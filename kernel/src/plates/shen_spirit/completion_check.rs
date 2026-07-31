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
    /// #15 · 本任务观察到的测试失败(生成定点提示后 drain)。
    pub test_failures: Vec<TestFailure>,
    /// #15 · 验证异常待回流标记(测试失败/Verifier 复核不通过)。
    /// 天盘每轮取一次(take_verification_anomaly),经 certainty_history
    /// 既有通道回流 Manas,不开旁路。
    pub anomaly_pending: bool,
}

// ── #15 · 测试命令识别与失败解析(验证闭环·神盘观测)────────────
//
// 识别 shell 调用中的测试命令并解析失败用例,全部同步纯计算(微秒级),
// 不阻塞主流程。模式表可扩展:新框架 = 新 TestKind 变体 + detect/parse
// 各一条臂。解析不出失败用例时降级为原始尾部摘录——观测宁可粗糙,不可缺席。

/// 测试框架类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestKind {
    /// cargo test / cargo nextest
    Cargo,
    /// pytest
    Pytest,
    /// go test
    Go,
    /// vitest(直接调用)
    Vitest,
    /// npm/pnpm/yarn/bun test、jest —— 具体框架以输出行为准(✕/●/FAIL 通用)。
    NpmLike,
}

impl TestKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pytest => "pytest",
            Self::Go => "go",
            Self::Vitest => "vitest",
            Self::NpmLike => "npm",
        }
    }
}

/// 识别 shell 命令文本中的测试命令(子串匹配,具体框架优先于通用脚本)。
pub fn detect_test_command(command: &str) -> Option<TestKind> {
    if command.contains("cargo nextest") || command.contains("cargo test") {
        return Some(TestKind::Cargo);
    }
    if command.contains("pytest") {
        return Some(TestKind::Pytest);
    }
    if command.contains("go test") {
        return Some(TestKind::Go);
    }
    if command.contains("vitest") {
        return Some(TestKind::Vitest);
    }
    if command.contains("jest") {
        return Some(TestKind::NpmLike);
    }
    for pm in ["pnpm", "npm", "yarn", "bun"] {
        if command.contains(&format!("{pm} test")) || command.contains(&format!("{pm} run test"))
        {
            return Some(TestKind::NpmLike);
        }
    }
    None
}

/// 一次测试命令的失败观测。
#[derive(Debug, Clone)]
pub struct TestFailure {
    /// 触发失败的命令(截断保存)。
    pub command: String,
    pub kind: TestKind,
    /// 解析出的失败用例名;解析不出为空(降级为尾部摘录)。
    pub failed_cases: Vec<String>,
    /// 解析失败时的原始输出尾部摘录。
    pub tail_excerpt: Option<String>,
}

/// 按命令类型解析输出中的失败用例名(去重、保序)。
fn parse_failed_cases(kind: TestKind, output: &str) -> Vec<String> {
    let mut cases: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let n = name.trim();
        if !n.is_empty() && !cases.iter().any(|c| c == n) {
            cases.push(n.to_string());
        }
    };
    match kind {
        TestKind::Cargo => {
            // 行式:"test foo::bar ... FAILED"
            for line in output.lines() {
                let t = line.trim();
                if t.starts_with("test ") && t.ends_with("FAILED") {
                    let name = t["test ".len()..t.len() - "FAILED".len()]
                        .trim()
                        .trim_end_matches('.')
                        .trim();
                    push(name);
                }
            }
            // "failures:" 段:后续缩进行为用例名(不含空格的单 token)。
            let mut in_failures = false;
            for line in output.lines() {
                let t = line.trim();
                if t == "failures:" {
                    in_failures = true;
                    continue;
                }
                if !in_failures {
                    continue;
                }
                if t.is_empty() {
                    in_failures = false;
                    continue;
                }
                if line.starts_with("    ") && !t.contains(char::is_whitespace) {
                    push(t);
                }
            }
        }
        TestKind::Pytest => {
            // 行式:"FAILED tests/test_x.py::test_y - AssertionError: ..."
            for line in output.lines() {
                if let Some(rest) = line.trim().strip_prefix("FAILED ") {
                    push(rest.split(" - ").next().unwrap_or(rest));
                }
            }
        }
        TestKind::Go => {
            // 行式:"--- FAIL: TestFoo (0.01s)"
            for line in output.lines() {
                if let Some(rest) = line.trim().strip_prefix("--- FAIL: ") {
                    push(rest.split_whitespace().next().unwrap_or(rest));
                }
            }
        }
        TestKind::Vitest | TestKind::NpmLike => {
            // vitest:"✕ case name"、"FAIL  src/x.test.ts";
            // jest:"✕ case"、"● Suite › case"、"FAIL path"。
            for line in output.lines() {
                let t = line.trim();
                for sym in ["✕", "×"] {
                    if let Some(rest) = t.strip_prefix(sym) {
                        push(rest);
                    }
                }
                if let Some(rest) = t.strip_prefix("FAIL ") {
                    push(rest);
                }
                if let Some(rest) = t.strip_prefix("● ") {
                    // "● Console" 是日志段标题,非用例。
                    if !rest.starts_with("Console") {
                        push(rest);
                    }
                }
            }
        }
    }
    cases
}

/// 原始输出尾部摘录(解析失败时的降级观测),按行数与字符数双上限。
fn tail_excerpt(output: &str) -> String {
    const MAX_LINES: usize = 12;
    const MAX_CHARS: usize = 1500;
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(MAX_LINES);
    let mut s = lines[start..].join("\n");
    if s.len() > MAX_CHARS {
        s = s[s.len() - MAX_CHARS..].to_string();
    }
    s
}

/// 生成定点修复提示(失败用例清单 + 建议聚焦点)。
///
/// 中性事实语言(儒家"信"的对抗性延伸是【自评】,不指责):只陈述观测到
/// 的信号与建议的下一步,不评判模型行为。
fn format_failure_reminder(f: &TestFailure) -> String {
    let mut out = format!(
        "[Verification] Test command failed: `{}` ({}).",
        f.command,
        f.kind.as_str()
    );
    if !f.failed_cases.is_empty() {
        out.push_str(&format!("\nFailed cases ({}):", f.failed_cases.len()));
        for case in &f.failed_cases {
            out.push_str(&format!("\n- {case}"));
        }
        out.push_str(
            "\nSuggested focus: re-run the failing cases individually for the full output, \
             fix them, then re-run the whole suite to confirm before wrapping up.",
        );
    } else {
        let tail = f.tail_excerpt.as_deref().unwrap_or("(no output)");
        out.push_str(&format!(
            "\nIndividual failing cases could not be parsed from the output; tail excerpt:\n```\n{tail}\n```\
             \nLocate the failure from the output above, fix it, then re-run to confirm."
        ));
    }
    out
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

    /// #15 · 摄入一次 shell 调用的测试命令观测(天盘在工具结账时调用,
    /// 命令文本只有调用方才可见——hook 事件不载入参)。
    ///
    /// 非测试命令直接返回;测试命令失败(解析到失败用例 / 退出码非零 /
    /// 工具级错误)则记录 TestFailure 并置验证异常标记;通过的测试命令
    /// 不产生信号(修复确认由后续观测自然覆盖)。
    pub fn ingest_test_command(&self, command: &str, output: &str, error: &Option<String>) {
        let Some(kind) = detect_test_command(command) else {
            return;
        };
        let failed_cases = parse_failed_cases(kind, output);
        let exit_nonzero = parse_exit_code(output).is_some_and(|c| c != 0);
        let failed = !failed_cases.is_empty() || exit_nonzero || error.is_some();
        if !failed {
            return;
        }
        let tail = if failed_cases.is_empty() {
            Some(tail_excerpt(output))
        } else {
            None
        };
        let mut v = self.vector.lock().unwrap_or_else(|e| e.into_inner());
        v.anomaly_pending = true;
        v.test_failures.push(TestFailure {
            command: crate::utils::truncate_chars(command, 120),
            kind,
            failed_cases,
            tail_excerpt: tail,
        });
    }

    /// #15 · 取走全部待注入的定点修复提示(drain——同一失败只提示一次)。
    pub fn take_test_failure_reminders(&self) -> Vec<String> {
        let mut v = self.vector.lock().unwrap_or_else(|e| e.into_inner());
        v.test_failures
            .drain(..)
            .map(|f| format_failure_reminder(&f))
            .collect()
    }

    /// #15 · 位识融合信号:取一次验证异常标记(drain)。天盘在确定度
    /// 评估后调用,为真则压低本轮写入 certainty_history 的确定度。
    pub fn take_verification_anomaly(&self) -> bool {
        let mut v = self.vector.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut v.anomaly_pending)
    }

    /// #15 · 外部观测到的验证异常(如 Verifier 子代理复核不通过,
    /// "Verdict: FAIL")——与测试失败同一回流通道。
    pub fn note_verification_anomaly(&self) {
        self.vector
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .anomaly_pending = true;
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

    // ── #15 · 测试命令识别与失败解析 ──────────────────────────

    #[test]
    fn detect_test_command_patterns() {
        assert_eq!(detect_test_command("cargo test --lib"), Some(TestKind::Cargo));
        assert_eq!(
            detect_test_command("cargo nextest run"),
            Some(TestKind::Cargo)
        );
        assert_eq!(detect_test_command("pytest -x tests/"), Some(TestKind::Pytest));
        assert_eq!(detect_test_command("go test ./..."), Some(TestKind::Go));
        assert_eq!(detect_test_command("npx vitest run"), Some(TestKind::Vitest));
        assert_eq!(detect_test_command("pnpm test"), Some(TestKind::NpmLike));
        assert_eq!(detect_test_command("npm run test"), Some(TestKind::NpmLike));
        assert_eq!(detect_test_command("yarn test"), Some(TestKind::NpmLike));
        assert_eq!(detect_test_command("npx jest"), Some(TestKind::NpmLike));
        // 非测试命令不识别。
        assert_eq!(detect_test_command("ls -la"), None);
        assert_eq!(detect_test_command("cargo build --release"), None);
        assert_eq!(detect_test_command("pnpm install"), None);
    }

    const CARGO_FAIL_OUTPUT: &str = "\
running 3 tests
test foo::tests::a ... ok
test foo::tests::b ... FAILED
test bar::tests::c ... FAILED

failures:

---- foo::tests::b stdout ----
thread 'foo::tests::b' panicked at src/foo.rs:10:

failures:
    foo::tests::b
    bar::tests::c

test result: FAILED. 1 passed; 2 failed; 0 ignored
[exit code: 101]";

    #[test]
    fn parse_cargo_failures_from_lines_and_failures_section() {
        let cases = parse_failed_cases(TestKind::Cargo, CARGO_FAIL_OUTPUT);
        assert_eq!(cases, ["foo::tests::b", "bar::tests::c"]);
    }

    #[test]
    fn parse_pytest_failures() {
        let out = "\
FAILED tests/test_x.py::test_y - AssertionError: assert 1 == 2
FAILED tests/test_z.py::test_w - ValueError
[exit code: 1]";
        let cases = parse_failed_cases(TestKind::Pytest, out);
        assert_eq!(cases, ["tests/test_x.py::test_y", "tests/test_z.py::test_w"]);
    }

    #[test]
    fn parse_go_and_vitest_failures() {
        let go = "--- FAIL: TestAdd (0.01s)\n--- FAIL: TestSub (0.00s)\nFAIL\tpkg\n[exit code: 1]";
        assert_eq!(parse_failed_cases(TestKind::Go, go), ["TestAdd", "TestSub"]);

        let vitest = " ✓ adds\n ✕ subtracts\nFAIL  src/math.test.ts\n[exit code: 1]";
        let cases = parse_failed_cases(TestKind::Vitest, vitest);
        assert_eq!(cases, ["subtracts", "src/math.test.ts"]);
    }

    #[test]
    fn ingest_passing_test_command_is_silent() {
        let cl = CompletionChecklist::new();
        cl.ingest_test_command("cargo test", "test result: ok. 3 passed\n[exit code: 0]", &None);
        assert!(cl.take_test_failure_reminders().is_empty());
        assert!(!cl.take_verification_anomaly());
        // 非测试命令同样静默。
        cl.ingest_test_command("ls", "boom\n[exit code: 1]", &None);
        assert!(cl.take_test_failure_reminders().is_empty());
    }

    #[test]
    fn ingest_failing_test_command_yields_pinpoint_reminder() {
        let cl = CompletionChecklist::new();
        cl.ingest_test_command("cargo test --lib", CARGO_FAIL_OUTPUT, &None);
        let reminders = cl.take_test_failure_reminders();
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].contains("cargo test --lib"), "{reminders:?}");
        assert!(reminders[0].contains("foo::tests::b"), "{reminders:?}");
        assert!(reminders[0].contains("bar::tests::c"), "{reminders:?}");
        assert!(reminders[0].contains("Suggested focus"), "{reminders:?}");
        // drain 后不再重复提示。
        assert!(cl.take_test_failure_reminders().is_empty());
        // 验证异常标记:取一次为真,再取为假(单次回流)。
        assert!(cl.take_verification_anomaly());
        assert!(!cl.take_verification_anomaly());
    }

    #[test]
    fn unparseable_failure_degrades_to_tail_excerpt() {
        let cl = CompletionChecklist::new();
        let out = "weird custom runner output\nsomething broke badly\n[exit code: 1]";
        cl.ingest_test_command("pytest", out, &None);
        let reminders = cl.take_test_failure_reminders();
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].contains("could not be parsed"), "{reminders:?}");
        assert!(reminders[0].contains("something broke badly"), "{reminders:?}");
    }

    #[test]
    fn note_verification_anomaly_sets_flag_once() {
        let cl = CompletionChecklist::new();
        assert!(!cl.take_verification_anomaly());
        cl.note_verification_anomaly();
        assert!(cl.take_verification_anomaly());
        assert!(!cl.take_verification_anomaly());
    }
}
