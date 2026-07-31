//! policy — 显式权限策略链 (N1)
//!
//! 首中即胜,顺序即安全语义:
//!
//! | 位次 | 策略名 | 类别 | 实现位置 |
//! |------|--------|------|----------|
//! | 1 | `deny_rule` | deny | [`PermissionMatrix::chain_check`] — `[security] deny_rules` 绝对拒绝,无任何豁免 |
//! | 2 | `path_sandbox` | deny | 现有 `verify_path`(blocked_prefixes + workspace 边界),调用点不变 |
//! | 3 | `session_approval` | allow | 会话级批准记忆,挂人盘 `SessionBus`,在 `UserConfirmation` 门处生效 |
//! | 4 | `command_policy` | deny | 现有 `verify_command`(allowlist/blocklist),调用点不变 |
//! | 5 | `sensitive_file` | ask | [`PermissionMatrix::chain_check`] — 敏感文件强制用户确认 |
//! | 6 | fallback | — | 链无命中,交还 GeJu 评估与八门分发(行为与现状一致) |
//!
//! 公理 4(单向收紧):拒绝类策略(1/2/4)恒优先于批准类(3)——批准记忆只
//! 豁免"询问"这一动作,绝不豁免任何拒绝;敏感文件强制 ask 只会把执行
//! 收紧为 Guarded+确认,绝不放松。位次 2/4 保持现有调用点,判定结果与
//! 重排前完全等价。

use serde_json::Value;

use super::PermissionMatrix;

/// 策略链裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainVerdict {
    /// 链无命中 — 交还既有 GeJu/八门流程。
    Pass,
    /// 命中拒绝类策略。
    Deny { policy: &'static str, reason: String },
    /// 命中强制询问策略(单向收紧:追加用户确认)。
    Ask { policy: &'static str, reason: String },
}

/// 从工具入参提取 deny 规则主体:(规则种类, 匹配对象)。
///
/// 种类对齐 kimi-code 规则 DSL:`shell`→`Bash`(命令串)、
/// `read_file`→`Read`(路径)、`write_file`/`patch_file`/`revert_file`→`Write`(路径);
/// 其余工具以工具名为种类、入参 JSON 为匹配对象。
fn subject(tool_name: &str, input: &Value) -> (String, String) {
    match tool_name {
        "shell" => (
            "Bash".into(),
            input["command"].as_str().unwrap_or("").to_string(),
        ),
        "read_file" => (
            "Read".into(),
            input["path"].as_str().unwrap_or("").to_string(),
        ),
        "write_file" | "patch_file" | "revert_file" => (
            "Write".into(),
            input["path"].as_str().unwrap_or("").to_string(),
        ),
        other => (other.to_string(), input.to_string()),
    }
}

/// 匹配单条 deny 规则,形如 `Bash(rm *)` / `Read(/etc/**)`。
fn match_deny_rule(rule: &str, kind: &str, subj: &str) -> bool {
    let (Some(open), Some(close)) = (rule.find('('), rule.rfind(')') ) else {
        tracing::warn!(rule, "deny rule malformed (expected `Kind(glob)`), skipped");
        return false;
    };
    if close <= open {
        tracing::warn!(rule, "deny rule malformed (expected `Kind(glob)`), skipped");
        return false;
    }
    if !rule[..open].trim().eq_ignore_ascii_case(kind) {
        return false;
    }
    match glob::Pattern::new(&rule[open + 1..close]) {
        Ok(p) => p.matches(subj),
        Err(e) => {
            tracing::warn!(?e, rule, "deny rule has invalid glob, skipped");
            false
        }
    }
}

/// 敏感文件判定:命中返回强制 ask 的理由,非敏感返回 None。
///
/// - `.env*`:豁免 example/sample/template(如 `.env.example`);
/// - SSH 私钥(`id_rsa`/`id_dsa`/`id_ecdsa`/`id_ed25519`):`.pub` 天然
///   不等于私钥名,即豁免。
pub fn sensitive_path_reason(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    let lower = name.to_lowercase();
    if lower.starts_with(".env")
        && !(lower.contains("example") || lower.contains("sample") || lower.contains("template"))
    {
        return Some(format!(
            "sensitive file '{name}' matches .env* — confirmation required"
        ));
    }
    const SSH_KEY_NAMES: [&str; 4] = ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];
    if SSH_KEY_NAMES.contains(&lower.as_str()) {
        return Some(format!(
            "sensitive file '{name}' is an SSH private key — confirmation required"
        ));
    }
    None
}

/// 批准记忆键:工具名 + 入参的确定性序列化(精确匹配,不做泛化——
/// 同会话内"同一命令/同一入参"才命中,绝不自动放宽到模式)。
pub fn approval_key(tool_name: &str, input: &Value) -> String {
    format!("{tool_name}:{input}")
}

impl PermissionMatrix {
    /// 策略链入口:位次 1(deny 规则)→ 位次 5(敏感文件强制 ask)。
    ///
    /// 位次 2/4(路径沙箱、命令策略)保持现有调用点(`sandbox_input` /
    /// `verify_command`),判定不变;位次 3(会话批准记忆)在人盘
    /// `UserConfirmation` 门处生效——三者在模块头顺序表中统一编号。
    pub fn chain_check(&self, tool_name: &str, input: &Value) -> ChainVerdict {
        // 位次 1:deny 规则 —— 绝对优先,命中即拒,无任何豁免。
        let (kind, subj) = subject(tool_name, input);
        for rule in &self.deny_rules {
            if match_deny_rule(rule, &kind, &subj) {
                return ChainVerdict::Deny {
                    policy: "deny_rule",
                    reason: format!("denied by configured rule '{rule}'"),
                };
            }
        }
        // 位次 5:敏感文件强制 ask(单向收紧;位次 2 路径沙箱未覆盖的场景补位)。
        if let Some(path) = input["path"].as_str()
            && let Some(reason) = sensitive_path_reason(path)
        {
            return ChainVerdict::Ask {
                policy: "sensitive_file",
                reason,
            };
        }
        ChainVerdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::qian_permission::{PathOp, PermissionMatrix, SandboxConfig, ShellPolicy};
    use std::path::PathBuf;

    fn make_matrix(deny_rules: Vec<String>) -> PermissionMatrix {
        let workspace_root = std::env::current_dir().unwrap();
        PermissionMatrix {
            sandbox: SandboxConfig {
                workspace_root: workspace_root.canonicalize().unwrap(),
                allowed_paths: vec![],
                blocked_prefixes: vec![".git".into(), ".env".into()],
            },
            shell_policy: ShellPolicy {
                allowlist: vec![],
                blocklist: vec![],
            },
            deny_rules,
            confirmation_timeout: std::time::Duration::from_secs(30),
            sandbox_mode: crate::palaces::kun_config::SandboxMode::Required,
            backup_dir: PathBuf::from(".jia/backups"),
            execution_sandbox: None,
        }
    }

    #[test]
    fn deny_rule_bash_glob_hits() {
        let m = make_matrix(vec!["Bash(rm *)".into()]);
        let v = m.chain_check("shell", &serde_json::json!({"command": "rm -rf /tmp/x"}));
        assert!(
            matches!(v, ChainVerdict::Deny { policy: "deny_rule", .. }),
            "Bash(rm *) must deny `rm -rf /tmp/x`: {v:?}"
        );
    }

    #[test]
    fn deny_rule_no_match_passes() {
        let m = make_matrix(vec!["Bash(rm *)".into(), "Read(/etc/**)".into()]);
        assert_eq!(
            m.chain_check("shell", &serde_json::json!({"command": "ls -la"})),
            ChainVerdict::Pass
        );
        assert_eq!(
            m.chain_check("read_file", &serde_json::json!({"path": "/etcetera/x"})),
            ChainVerdict::Pass
        );
    }

    #[test]
    fn deny_rule_read_path_hits() {
        let m = make_matrix(vec!["Read(/etc/**)".into()]);
        let v = m.chain_check("read_file", &serde_json::json!({"path": "/etc/passwd"}));
        assert!(matches!(v, ChainVerdict::Deny { policy: "deny_rule", .. }));
    }

    #[test]
    fn deny_rule_kind_mismatch_passes() {
        // Read 规则不约束 Bash 主体
        let m = make_matrix(vec!["Read(/etc/**)".into()]);
        assert_eq!(
            m.chain_check("shell", &serde_json::json!({"command": "cat /etc/passwd"})),
            ChainVerdict::Pass
        );
    }

    #[test]
    fn deny_rule_beats_sensitive_ask() {
        // 链顺序:deny(位次 1)先于敏感文件 ask(位次 5)
        let m = make_matrix(vec!["Read(/work/**)".into()]);
        let v = m.chain_check("read_file", &serde_json::json!({"path": "/work/.env"}));
        assert!(
            matches!(v, ChainVerdict::Deny { policy: "deny_rule", .. }),
            "deny rule must win over sensitive_file ask: {v:?}"
        );
    }

    #[test]
    fn malformed_rule_skipped_not_fatal() {
        let m = make_matrix(vec!["no-parens".into(), "Bash(".into()]);
        assert_eq!(
            m.chain_check("shell", &serde_json::json!({"command": "ls"})),
            ChainVerdict::Pass
        );
    }

    #[test]
    fn sensitive_env_variants() {
        assert!(sensitive_path_reason(".env").is_some());
        assert!(sensitive_path_reason("/work/.env.local").is_some());
        assert!(sensitive_path_reason("/work/.env.production").is_some());
        // 豁免:example/sample/template
        assert!(sensitive_path_reason(".env.example").is_none());
        assert!(sensitive_path_reason("/work/.env.sample").is_none());
        assert!(sensitive_path_reason(".env.template").is_none());
    }

    #[test]
    fn sensitive_ssh_keys() {
        assert!(sensitive_path_reason("/home/u/.ssh/id_rsa").is_some());
        assert!(sensitive_path_reason("id_ed25519").is_some());
        // 豁免:.pub 公钥
        assert!(sensitive_path_reason("/home/u/.ssh/id_rsa.pub").is_none());
        assert!(sensitive_path_reason("/home/u/.ssh/id_ed25519.pub").is_none());
    }

    #[test]
    fn non_sensitive_passes() {
        assert!(sensitive_path_reason("Cargo.toml").is_none());
        assert!(sensitive_path_reason("/work/src/main.rs").is_none());
    }

    #[test]
    fn sensitive_file_ask_via_chain() {
        let m = make_matrix(vec![]);
        let v = m.chain_check("read_file", &serde_json::json!({"path": "/work/.env.secret"}));
        assert!(
            matches!(v, ChainVerdict::Ask { policy: "sensitive_file", .. }),
            "sensitive file must force ask: {v:?}"
        );
    }

    #[test]
    fn approval_key_is_exact() {
        let a = approval_key("shell", &serde_json::json!({"command": "ls"}));
        let b = approval_key("shell", &serde_json::json!({"command": "ls"}));
        let c = approval_key("shell", &serde_json::json!({"command": "ls -la"}));
        assert_eq!(a, b);
        assert_ne!(a, c, "different input must not share an approval key");
    }

    #[test]
    fn deny_rule_write_glob_hits_revert_file() {
        // revert_file 与 write_file/patch_file 同属 Write 主体(己仪写域):
        // Write(path) deny 规则必须同样命中它。
        let m = make_matrix(vec!["Write(/protected/**)".into()]);
        for tool in ["write_file", "patch_file", "revert_file"] {
            let v = m.chain_check(tool, &serde_json::json!({"path": "/protected/x.txt"}));
            assert!(
                matches!(v, ChainVerdict::Deny { policy: "deny_rule", .. }),
                "Write(/protected/**) must deny {tool}: {v:?}"
            );
        }
        // 未命中路径仍放行。
        assert_eq!(
            m.chain_check("revert_file", &serde_json::json!({"path": "/work/x.txt"})),
            ChainVerdict::Pass
        );
    }

    #[test]
    fn existing_path_check_unaffected_by_chain() {
        // 行为等价:链放行后,现有 verify_path 判定保持原样。
        let m = make_matrix(vec![]);
        assert!(m.verify_path("/etc/passwd", PathOp::Read).is_err());
        assert_eq!(
            m.chain_check("read_file", &serde_json::json!({"path": "Cargo.toml"})),
            ChainVerdict::Pass
        );
    }
}
