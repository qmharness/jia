//! verifier — Verifier 子代理 shell 命令白名单(迭代四 #15 硬约束)
//!
//! Verifier 注册表结构性缺席写工具,但 shell 本身可执行任意命令——
//! "只读"此前仅靠身份提示词软约束。本模块把它落为门禁硬约束:Verifier
//! 的 shell 调用逐段(`&&`/`|` 分段)过本白名单,默认拒绝其他一切。
//! 只收紧不放松:主 agent 与其余子代理的 shell 不经此表(本校验挂在
//! 人盘的子代理实例上,见 ren_human::HumanPlate::verifier_shell_only),
//! verify_command 全局行为不变。

/// Verifier shell 白名单校验。`cmd` 为 shell 工具的 `command` 入参。
///
/// 规则:
/// 1. 重定向 `<`/`>`(引号外)一律拒绝——全局元字符规则不覆盖它们,
///    而 `echo > x` / `cargo test > out` 皆是写;
/// 2. 按 `&&` 与 `|` 逐段校验(引号内不分段),每段首词(去路径)命中
///    模式表才放行,含子命令收窄(`git push` vs `git status`);
/// 3. 默认拒绝。
pub fn verify_verifier_command(cmd: &str) -> Result<(), String> {
    if let Some(c) = redirection_metachar(cmd) {
        return Err(denied(&format!("redirection '{c}' is not allowed")));
    }
    for segment in split_segments(cmd) {
        let tokens = shell_words::split(&segment)
            .map_err(|e| denied(&format!("unparseable command segment ({e})")))?;
        let Some(first) = tokens.first() else {
            continue;
        };
        let name = first.rsplit('/').next().unwrap_or(first);
        if !segment_allowed(name, &tokens[1..]) {
            return Err(denied(&format!(
                "command '{name}' is not in the verification allowlist"
            )));
        }
    }
    Ok(())
}

/// 拒绝消息:引导模型改用验证命令,写操作交主 agent。
fn denied(what: &str) -> String {
    format!(
        "{what} — Verifier 只执行验证命令(测试/构建/lint/只读查看),写操作请由主 agent 执行"
    )
}

/// 单段命令判定:`name` 为去路径后的命令名,`args` 为其参数。
fn segment_allowed(name: &str, args: &[String]) -> bool {
    let sub = args.first().map(String::as_str);
    match name {
        // 只读查看命令(echo 在重定向被禁后只写 stdout,只读)。
        "ls" | "cat" | "grep" | "rg" | "find" | "wc" | "head" | "tail" | "pwd" | "echo" => true,
        // 测试 / 构建 / lint:整体放行。
        "pytest" | "vitest" | "jest" => true,
        // 子命令收窄。
        "cargo" => matches!(sub, Some("test" | "check" | "clippy" | "build")),
        "go" => matches!(sub, Some("test")),
        "git" => matches!(sub, Some("status" | "diff" | "log" | "show" | "blame")),
        "npm" | "pnpm" | "yarn" | "bun" => matches!(sub, Some("test")),
        _ => false,
    }
}

/// 引号外按 `&&` 与 `|` 分段(引号内的 `|` 是字面量,如
/// `git log --format="%H | %s"`,不分段)。
fn split_segments(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_single => {
                current.push(c);
                escaped = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }
            '|' if !in_single && !in_double => {
                segments.push(std::mem::take(&mut current));
            }
            '&' if !in_single && !in_double && chars.peek() == Some(&'&') => {
                chars.next();
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
}

/// 引号外的重定向元字符 `<`/`>`(引号内字面量放行)。
fn redirection_metachar(cmd: &str) -> Option<char> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for c in cmd.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '<' | '>' if !in_single && !in_double => return Some(c),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_verification_commands() {
        for cmd in [
            "cargo test",
            "cargo test -p kernel --lib",
            "cargo check --all-targets",
            "cargo clippy -- -D warnings",
            "cargo build",
            "pytest tests/",
            "go test ./...",
            "vitest run",
            "jest",
            "npm test",
            "pnpm test",
            "yarn test",
            "bun test",
            "git status",
            "git diff HEAD~1",
            "git log --oneline -5",
            "git show abc123",
            "git blame src/lib.rs",
            "ls -la",
            "cat Cargo.toml",
            "grep -rn foo src",
            "rg pattern",
            "find . -name '*.rs'",
            "wc -l file",
            "head -20 file",
            "tail -f log",
        ] {
            assert!(verify_verifier_command(cmd).is_ok(), "{cmd} must pass");
        }
    }

    #[test]
    fn denies_everything_else_by_default() {
        for cmd in [
            "rm -rf /tmp/x",
            "rm x",
            "git push",
            "git commit -m x",
            "git checkout main",
            "cargo run",
            "cargo fmt",
            "npm install",
            "npm run build",
            "go build",
            "mv a b",
            "touch x",
            "sed -i s/a/b/ f",
            "curl http://x",
            "make test",
        ] {
            let err = verify_verifier_command(cmd).unwrap_err();
            assert!(
                err.contains("写操作请由主 agent 执行"),
                "{cmd}: denial must guide, got: {err}"
            );
        }
    }

    #[test]
    fn denies_redirection_even_for_allowed_commands() {
        for cmd in ["echo hi > x", "echo > x", "cargo test > /tmp/out", "cat f >> g"] {
            assert!(verify_verifier_command(cmd).is_err(), "{cmd} must be denied");
        }
        // 引号内的 > 是字面量,不误伤。
        assert!(verify_verifier_command("echo \"a > b\"").is_ok());
    }

    #[test]
    fn validates_each_and_and_pipe_segment() {
        // 全段合规 → 放行
        assert!(verify_verifier_command("cargo test && git status").is_ok());
        // && 第二段不合规 → 拒(防 `cargo test && rm x` 绕过)
        let err = verify_verifier_command("cargo test && rm x").unwrap_err();
        assert!(err.contains("'rm'"), "{err}");
        // 管道逐段:tee 段被拒
        let err = verify_verifier_command("cargo test | tee /tmp/x").unwrap_err();
        assert!(err.contains("'tee'"), "{err}");
        // 合规管道放行(管道本身由全局元字符规则另行处理,此表只管命令名)
        assert!(verify_verifier_command("cargo test | tail -5").is_ok());
        // 引号内 | 不分段
        assert!(verify_verifier_command("git log --format=\"%H | %s\"").is_ok());
    }

    #[test]
    fn unparseable_segment_is_denied() {
        assert!(verify_verifier_command("cargo test \"unclosed").is_err());
    }
}
