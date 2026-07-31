//! 完成检查清单 CompletionChecklist — 神盘 hook 扩展·确定性信号。
//!
//! ConfidentStop 时用确定性信号（非 LLM 判断）辅助确认任务产物存在。
//! 纯正则 + 文件系统检查，无 LLM。不确定时升级为 ask_user。

use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

use std::collections::HashMap;
use std::collections::VecDeque;
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
///
/// 多会话隔离:可变状态按 session_id 分桶(挂法参照 SessionBus 的
/// per-session 桶先例)——天盘 loop 摄入/取走都带本会话 id,A 会话的
/// 测试失败提示/异常标记不会被 B 会话取走;子代理会话(ephemeral)以
/// 各自 loop id 天然分桶。纯函数部分(测试命令识别/失败解析/提示格式化)
/// 共享无状态。会话结束经 `end_session` 回收(rin 断连清扫挂点)。
///
/// 例外:hook 观测向量(exit_codes / files_created / grep_matches,仅供
/// ConfidentStop `assess` 使用)挂固定桶——HookEvent::ToolPostExecute
/// 不携带 session_id,无法按会话归属;该路径保持隔离前的进程级语义。
pub struct CompletionChecklist {
    vectors: Mutex<SessionVectors>,
}

/// 会话状态表带界上限:超出时逐出最久未触会话(LRU),防长驻进程内存
/// 无界增长(参照 SessionBus 各桶经 rin 断连清扫回收,此处再兜底一层)。
const MAX_SESSION_VECTORS: usize = 64;

/// hook 观测桶的固定键(见 CompletionChecklist 文档"例外"段)。
const HOOK_BUCKET: &str = "__hook__";

/// per-session 桶 + 触序(同一把锁内维护,map 与 order 始终一致)。
#[derive(Default)]
struct SessionVectors {
    map: HashMap<String, CompletionVector>,
    /// 触序:队首 = 最久未触;任何读写都把 key 挪到队尾。
    order: VecDeque<String>,
}

impl SessionVectors {
    fn touch(&mut self, session_id: &str) {
        self.order.retain(|k| k != session_id);
        self.order.push_back(session_id.to_string());
    }

    /// 写路径取桶:不存在则创建;满界时先逐出最久未触会话。
    fn bucket(&mut self, session_id: &str) -> &mut CompletionVector {
        if !self.map.contains_key(session_id) {
            while self.map.len() >= MAX_SESSION_VECTORS {
                let Some(oldest) = self.order.pop_front() else {
                    break;
                };
                self.map.remove(&oldest);
            }
            self.map
                .insert(session_id.to_string(), CompletionVector::default());
        }
        self.touch(session_id);
        self.map.get_mut(session_id).expect("bucket just ensured")
    }

    /// 读路径取桶:不为缺失会话建桶(读不制造残留)。
    fn bucket_if_present(&mut self, session_id: &str) -> Option<&mut CompletionVector> {
        if !self.map.contains_key(session_id) {
            return None;
        }
        self.touch(session_id);
        self.map.get_mut(session_id)
    }

    fn remove(&mut self, session_id: &str) {
        self.map.remove(session_id);
        self.order.retain(|k| k != session_id);
    }
}

impl CompletionChecklist {
    pub fn new() -> Self {
        Self {
            vectors: Mutex::new(SessionVectors::default()),
        }
    }

    /// 从 ToolPostExecute 事件解析结构化信号(hook 路径,挂固定桶——
    /// hook 事件不携带 session_id,见结构体文档"例外"段)。
    pub fn ingest(&self, tool_name: &str, output: &str, error: &Option<String>) {
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        let v = sv.bucket(HOOK_BUCKET);

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

    /// ConfidentStop 时评估完成度(hook 固定桶)。
    pub fn assess(&self) -> CompletionAssessment {
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        let mut missing = Vec::new();

        if let Some(v) = sv.bucket_if_present(HOOK_BUCKET) {
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

    /// Reset the accumulated hook-bucket vector for a new task.
    pub fn reset(&self) {
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        *sv.bucket(HOOK_BUCKET) = CompletionVector::default();
    }

    /// #15 · 摄入一次 shell 调用的测试命令观测(天盘在工具结账时调用,
    /// 命令文本只有调用方才可见——hook 事件不载入参)。按 session_id
    /// 分桶:多会话并发时各自累积,互不串扰。
    ///
    /// 非测试命令直接返回;测试命令失败(解析到失败用例 / 退出码非零 /
    /// 工具级错误)则记录 TestFailure 并置验证异常标记;通过的测试命令
    /// 不产生信号(修复确认由后续观测自然覆盖)。
    pub fn ingest_test_command(
        &self,
        session_id: &str,
        command: &str,
        output: &str,
        error: &Option<String>,
    ) {
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
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        let v = sv.bucket(session_id);
        v.anomaly_pending = true;
        v.test_failures.push(TestFailure {
            command: crate::utils::truncate_chars(command, 120),
            kind,
            failed_cases,
            tail_excerpt: tail,
        });
    }

    /// #15 · 取走该会话全部待注入的定点修复提示(drain——同一失败只
    /// 提示一次)。只读本会话的桶,其他会话的待注入提示不受影响。
    pub fn take_test_failure_reminders(&self, session_id: &str) -> Vec<String> {
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = sv.bucket_if_present(session_id) else {
            return Vec::new();
        };
        v.test_failures
            .drain(..)
            .map(|f| format_failure_reminder(&f))
            .collect()
    }

    /// #15 · 位识融合信号:取一次该会话的验证异常标记(drain)。天盘在
    /// 确定度评估后调用,为真则压低本轮写入 certainty_history 的确定度。
    pub fn take_verification_anomaly(&self, session_id: &str) -> bool {
        let mut sv = self.vectors.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = sv.bucket_if_present(session_id) else {
            return false;
        };
        std::mem::take(&mut v.anomaly_pending)
    }

    /// #15 · 外部观测到的验证异常(如 Verifier 子代理复核不通过,
    /// "Verdict: FAIL")——与测试失败同一回流通道,记到发起复核的
    /// 会话桶。
    pub fn note_verification_anomaly(&self, session_id: &str) {
        self.vectors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bucket(session_id)
            .anomaly_pending = true;
    }

    /// 会话结束回收该会话的桶(rin 断连清扫挂点;LRU 超界逐出是兜底)。
    pub fn end_session(&self, session_id: &str) {
        self.vectors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
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
        cl.ingest_test_command(
            "s1",
            "cargo test",
            "test result: ok. 3 passed\n[exit code: 0]",
            &None,
        );
        assert!(cl.take_test_failure_reminders("s1").is_empty());
        assert!(!cl.take_verification_anomaly("s1"));
        // 非测试命令同样静默。
        cl.ingest_test_command("s1", "ls", "boom\n[exit code: 1]", &None);
        assert!(cl.take_test_failure_reminders("s1").is_empty());
    }

    #[test]
    fn ingest_failing_test_command_yields_pinpoint_reminder() {
        let cl = CompletionChecklist::new();
        cl.ingest_test_command("s1", "cargo test --lib", CARGO_FAIL_OUTPUT, &None);
        let reminders = cl.take_test_failure_reminders("s1");
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].contains("cargo test --lib"), "{reminders:?}");
        assert!(reminders[0].contains("foo::tests::b"), "{reminders:?}");
        assert!(reminders[0].contains("bar::tests::c"), "{reminders:?}");
        assert!(reminders[0].contains("Suggested focus"), "{reminders:?}");
        // drain 后不再重复提示。
        assert!(cl.take_test_failure_reminders("s1").is_empty());
        // 验证异常标记:取一次为真,再取为假(单次回流)。
        assert!(cl.take_verification_anomaly("s1"));
        assert!(!cl.take_verification_anomaly("s1"));
    }

    #[test]
    fn unparseable_failure_degrades_to_tail_excerpt() {
        let cl = CompletionChecklist::new();
        let out = "weird custom runner output\nsomething broke badly\n[exit code: 1]";
        cl.ingest_test_command("s1", "pytest", out, &None);
        let reminders = cl.take_test_failure_reminders("s1");
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].contains("could not be parsed"), "{reminders:?}");
        assert!(reminders[0].contains("something broke badly"), "{reminders:?}");
    }

    #[test]
    fn note_verification_anomaly_sets_flag_once() {
        let cl = CompletionChecklist::new();
        assert!(!cl.take_verification_anomaly("s1"));
        cl.note_verification_anomaly("s1");
        assert!(cl.take_verification_anomaly("s1"));
        assert!(!cl.take_verification_anomaly("s1"));
    }

    // ── 多会话隔离 ────────────────────────────────────────────

    /// 两个会话并发摄入测试失败(线程交错),各自只取到自己的提示;
    /// 异常标记按会话独立,单次取走语义保持。
    #[test]
    fn concurrent_sessions_drain_only_their_own() {
        let cl = std::sync::Arc::new(CompletionChecklist::new());
        let mut handles = Vec::new();
        for (sid, case) in [("sess-a", "foo::a"), ("sess-b", "foo::b")] {
            let cl = cl.clone();
            handles.push(std::thread::spawn(move || {
                let out = format!("test {case} ... FAILED\n[exit code: 101]");
                for _ in 0..16 {
                    cl.ingest_test_command(sid, "cargo test", &out, &None);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let a = cl.take_test_failure_reminders("sess-a");
        let b = cl.take_test_failure_reminders("sess-b");
        assert_eq!(a.len(), 16, "sess-a drains exactly its own: {a:?}");
        assert_eq!(b.len(), 16, "sess-b drains exactly its own: {b:?}");
        assert!(
            a.iter().all(|r| r.contains("foo::a") && !r.contains("foo::b")),
            "no cross-talk into sess-a: {a:?}"
        );
        assert!(
            b.iter().all(|r| r.contains("foo::b") && !r.contains("foo::a")),
            "no cross-talk into sess-b: {b:?}"
        );
        // drain 是单次的;异常标记各自独立取走。
        assert!(cl.take_test_failure_reminders("sess-a").is_empty());
        assert!(cl.take_verification_anomaly("sess-a"));
        assert!(cl.take_verification_anomaly("sess-b"));
        assert!(!cl.take_verification_anomaly("sess-a"));
        assert!(!cl.take_verification_anomaly("sess-b"));
    }

    /// 单会话语义回归:同会话多次摄入按序聚合,一次 drain 取净。
    #[test]
    fn single_session_aggregates_then_drains_once() {
        let cl = CompletionChecklist::new();
        cl.ingest_test_command("s1", "cargo test --lib", CARGO_FAIL_OUTPUT, &None);
        cl.ingest_test_command("s1", "pytest", "FAILED t.py::test_x - E\n[exit code: 1]", &None);
        let reminders = cl.take_test_failure_reminders("s1");
        assert_eq!(reminders.len(), 2);
        assert!(reminders[0].contains("foo::tests::b"), "{reminders:?}");
        assert!(reminders[1].contains("t.py::test_x"), "{reminders:?}");
        assert!(cl.take_test_failure_reminders("s1").is_empty());
    }

    /// end_session 回收该会话的桶;其他会话不受影响。
    #[test]
    fn end_session_drops_only_that_session() {
        let cl = CompletionChecklist::new();
        cl.ingest_test_command("s1", "cargo test --lib", CARGO_FAIL_OUTPUT, &None);
        cl.ingest_test_command("s2", "cargo test --lib", CARGO_FAIL_OUTPUT, &None);
        cl.end_session("s1");
        assert!(cl.take_test_failure_reminders("s1").is_empty());
        assert!(!cl.take_verification_anomaly("s1"));
        assert_eq!(cl.take_test_failure_reminders("s2").len(), 1);
        assert!(cl.take_verification_anomaly("s2"));
    }

    /// 带界 LRU:会话桶超过上限时逐出最久未触会话(内存不随会话数
    /// 无界增长);被逐会话的待取信号随之丢弃。
    #[test]
    fn session_vectors_lru_evicts_oldest_beyond_cap() {
        let cl = CompletionChecklist::new();
        for i in 0..MAX_SESSION_VECTORS {
            cl.note_verification_anomaly(&format!("s{i}"));
        }
        // 触一次 s0(取走即视为活跃),使其不再是最久未触。
        assert!(cl.take_verification_anomaly("s0"));
        // 第 cap+1 个会话触发逐出:最久未触的 s1 被逐,s0 存活。
        cl.note_verification_anomaly("overflow");
        assert!(!cl.take_verification_anomaly("s1"), "oldest untouched evicted");
        assert!(cl.take_verification_anomaly("overflow"));
        // s0 桶仍在(标记已被上面取走,这里验证桶未被逐:重新标记可取)。
        cl.note_verification_anomaly("s0");
        assert!(cl.take_verification_anomaly("s0"));
    }
}
