//! 完成检查清单 CompletionChecklist — 神盘 hook 扩展·确定性信号。
//!
//! ConfidentStop 时用确定性信号（非 LLM 判断）辅助确认任务产物存在。
//! 纯正则 + 文件系统检查，无 LLM。不确定时升级为 ask_user。

use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

use std::path::PathBuf;
use std::sync::Mutex;

/// 确定性完成信号累积向量。
#[derive(Debug, Clone, Default)]
pub struct CompletionVector {
    pub exit_codes: Vec<i32>,
    pub files_created: Vec<PathBuf>,
    pub grep_matches: Vec<usize>,
}

/// 完成度评估结果。
#[derive(Debug, Clone)]
pub enum CompletionAssessment {
    /// 所有信号正常——静默通过。
    SilentPass,
    /// 存在异常信号——升级为用户确认。
    UpgradeToUser {
        reason: String,
        missing: Vec<String>,
    },
}

/// 完成检查清单——神盘 hook 观测，纯确定性逻辑。
pub struct CompletionChecklist {
    vector: Mutex<CompletionVector>,
}

impl CompletionChecklist {
    pub fn new() -> Self {
        Self {
            vector: Mutex::new(CompletionVector::default()),
        }
    }

    /// 从 ToolPostExecute 事件解析结构化信号。
    pub fn ingest(&self, tool_name: &str, output: &str, error: &Option<String>) {
        let mut v = self.vector.lock().unwrap_or_else(|e| e.into_inner());

        // Parse sandbox exit code from output string: "[exit code: N]"
        if let Some(code) = parse_exit_code(output) {
            v.exit_codes.push(code);
        }

        // Track write_file targets
        if tool_name == "write_file" {
            if let Some(path) = extract_file_path(output) {
                v.files_created.push(PathBuf::from(path));
            }
        }

        // Track grep match counts
        if tool_name == "grep" {
            if error.is_none() {
                v.grep_matches.push(output.lines().count());
            }
        }
    }

    /// ConfidentStop 时评估完成度。
    pub fn assess(&self) -> CompletionAssessment {
        let v = self.vector.lock().unwrap_or_else(|e| e.into_inner());
        let mut missing = Vec::new();

        // Check all exit codes
        let failures: Vec<_> = v.exit_codes.iter().filter(|&&c| c != 0).collect();
        if !failures.is_empty() {
            missing.push(format!(
                "{} shell command(s) failed (exit ≠ 0)",
                failures.len()
            ));
        }

        // Check files actually exist on disk
        for path in &v.files_created {
            if !path.exists() {
                missing.push(format!("claimed file not found: {}", path.display()));
            }
        }

        if missing.is_empty() {
            CompletionAssessment::SilentPass
        } else {
            CompletionAssessment::UpgradeToUser {
                reason: format!("completion checklist found {} issue(s)", missing.len()),
                missing,
            }
        }
    }

    /// Reset the accumulated vector for a new task.
    pub fn reset(&self) {
        *self.vector.lock().unwrap_or_else(|e| e.into_inner()) = CompletionVector::default();
    }
}

/// Parse sandbox output "[exit code: N]" pattern.
fn parse_exit_code(output: &str) -> Option<i32> {
    let marker = "[exit code:";
    if let Some(pos) = output.rfind(marker) {
        let rest = &output[pos + marker.len()..];
        if let Some(end) = rest.find(']') {
            return rest[..end].trim().parse().ok();
        }
    }
    None
}

/// Extract file path from tool output (write_file typically echoes the path).
fn extract_file_path(output: &str) -> Option<&str> {
    let trimmed = output.trim();
    if trimmed.starts_with('/') || trimmed.starts_with("./") {
        Some(trimmed)
    } else {
        None
    }
}

/// Hook wrapper that feeds ToolPostExecute events into CompletionChecklist.
pub struct CompletionCheckHook {
    checklist: std::sync::Arc<CompletionChecklist>,
}

impl CompletionCheckHook {
    pub fn new(checklist: std::sync::Arc<CompletionChecklist>) -> Self {
        Self { checklist }
    }
}

#[async_trait]
impl Hook for CompletionCheckHook {
    fn name(&self) -> &str {
        "completion_check"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::BaiHu]
    }
    fn matcher(&self) -> Option<&str> {
        Some("shell|write_file|grep|read_file")
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        if let HookEvent::ToolPostExecute {
            tool_name,
            output,
            error,
            ..
        } = &event
        {
            self.checklist.ingest(tool_name, output, error);
        }
        HookResult::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exit_code_zero() {
        assert_eq!(parse_exit_code("ok\n[exit code: 0]"), Some(0));
    }

    #[test]
    fn parse_exit_code_nonzero() {
        assert_eq!(parse_exit_code("error\n[exit code: 1]"), Some(1));
    }

    #[test]
    fn parse_exit_code_none() {
        assert_eq!(parse_exit_code("no exit code here"), None);
    }

    #[test]
    fn silent_pass_when_all_clean() {
        let cl = CompletionChecklist::new();
        cl.ingest("shell", "output\n[exit code: 0]", &None);
        assert!(matches!(cl.assess(), CompletionAssessment::SilentPass));
    }

    #[test]
    fn upgrade_when_exit_nonzero() {
        let cl = CompletionChecklist::new();
        cl.ingest("shell", "fail\n[exit code: 1]", &None);
        assert!(matches!(
            cl.assess(),
            CompletionAssessment::UpgradeToUser { .. }
        ));
    }
}
