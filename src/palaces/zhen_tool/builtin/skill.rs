use std::sync::Arc;
use crate::error::ToolError;
use std::sync::RwLock;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::li_skill::SkillRegistry;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, CommunicateAction};

/// A tool that bridges the ToolRegistry and SkillRegistry, allowing the LLM
/// to invoke skills by name. When called, returns the skill's prompt content.
///
/// The description is built dynamically on each `description()` call so it
/// stays in sync with file-watcher hot-reloads.
pub struct SkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }

    fn build_description(registry: &Arc<RwLock<SkillRegistry>>) -> String {
        let reg = registry.read().unwrap_or_else(|e| e.into_inner());
        let names: Vec<_> = reg.list_names();
        if names.is_empty() {
            "Invoke a specialized skill by name. No skills currently loaded.".into()
        } else {
            let list: Vec<_> = names.iter().map(|n| n.as_str()).collect();
            format!(
                "Invoke a specialized skill by name. Available skills: {}. \
                 Call with a skill name to get detailed instructions for that domain.",
                list.join(", ")
            )
        }
    }
}

#[async_trait]
impl BaseTool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> String {
        Self::build_description(&self.registry)
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren(CommunicateAction {
            endpoint: "skill".into(),
            payload: String::new(),
        })
    }

    fn target_palace(&self, _input: &Value) -> crate::palaces::Palace {
        crate::palaces::Palace::Zhong
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The skill name to invoke (e.g., 'code-review')"
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments to pass to the skill"
                },
                "script": {
                    "type": "string",
                    "description": "Read a bundled script from the skill's scripts/ directory by filename."
                },
                "reference": {
                    "type": "string",
                    "description": "Read a bundled reference doc from the skill's references/ directory by filename."
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let skill_name = input["skill"].as_str().ok_or("Missing 'skill' parameter")?;

        let reg = self.registry.read().unwrap_or_else(|e| e.into_inner());
        let skill = reg.get(skill_name).ok_or_else(|| {
            let names: Vec<_> = reg.list_names();
            let available = if names.is_empty() {
                "no skills loaded".into()
            } else {
                names
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!("Unknown skill '{}'. Available: {}", skill_name, available)
        })?;

        // Serve a bundled script
        if let Some(script_name) = input["script"].as_str() {
            return Ok(skill.scripts.get(script_name).cloned().ok_or_else(|| {
                let available: Vec<_> = skill.scripts.keys().map(|k| k.as_str()).collect();
                if available.is_empty() {
                    format!("Skill '{}' has no bundled scripts.", skill_name)
                } else {
                    format!(
                        "Script '{}' not found in skill '{}'. Available: {}",
                        script_name,
                        skill_name,
                        available.join(", ")
                    )
                }
            })?);
        }

        // Serve a bundled reference
        if let Some(ref_name) = input["reference"].as_str() {
            return Ok(skill.references.get(ref_name).cloned().ok_or_else(|| {
                let available: Vec<_> = skill.references.keys().map(|k| k.as_str()).collect();
                if available.is_empty() {
                    format!("Skill '{}' has no bundled references.", skill_name)
                } else {
                    format!(
                        "Reference '{}' not found in skill '{}'. Available: {}",
                        ref_name,
                        skill_name,
                        available.join(", ")
                    )
                }
            })?);
        }

        // Default: return the skill prompt
        let mut content = skill.prompt.clone();
        if let Some(ref emph) = skill.emphasis {
            content.push_str("\n\n> **Critical Reminder:**\n> ");
            content.push_str(&emph.replace('\n', "\n> "));
        }

        // Append bundled file index if present
        if !skill.scripts.is_empty() || !skill.references.is_empty() {
            content.push_str("\n\n---\n## Bundled Resources\n");
            if !skill.scripts.is_empty() {
                let names: Vec<_> = skill.scripts.keys().map(|k| k.as_str()).collect();
                content.push_str(&format!(
                    "\n**Scripts** (use `skill({}, script=\"...\")` to read): {}",
                    skill_name,
                    names.join(", ")
                ));
            }
            if !skill.references.is_empty() {
                let names: Vec<_> = skill.references.keys().map(|k| k.as_str()).collect();
                content.push_str(&format!(
                    "\n**References** (use `skill({}, reference=\"...\")` to read): {}",
                    skill_name,
                    names.join(", ")
                ));
            }
            content.push('\n');
        }

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
        }
    }

    use super::*;
    use crate::palaces::li_skill::{Skill, SkillRegistry};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn test_registry() -> Arc<RwLock<SkillRegistry>> {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "code-review".into(),
            description: "Reviews code".into(),
            prompt: "Check for SQL injection.".into(),
            source_path: PathBuf::from("skills/code-review.md"),
            always: false,
            paths: None,
            emphasis: None,
            auto_evolve: false,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        });
        reg.register(Skill {
            name: "safety".into(),
            description: "Safety rules".into(),
            prompt: "Always verify commands.".into(),
            source_path: PathBuf::from("skills/safety.md"),
            always: false,
            paths: None,
            emphasis: None,
            auto_evolve: false,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        });
        Arc::new(RwLock::new(reg))
    }

    #[tokio::test]
    async fn skill_tool_lists_available_skills() {
        let tool = SkillTool::new(test_registry());
        assert!(tool.description().to_string().contains("code-review"));
        assert!(tool.description().to_string().contains("safety"));
    }

    #[tokio::test]
    async fn skill_tool_execute_returns_prompt() {
        let tool = SkillTool::new(test_registry());
        let result = tool
            .execute(serde_json::json!({"skill": "code-review"}), &test_ctx())
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().to_string().contains("SQL injection"));
    }

    #[tokio::test]
    async fn skill_tool_unknown_skill_returns_error_with_list() {
        let tool = SkillTool::new(test_registry());
        let result = tool
            .execute(serde_json::json!({"skill": "nonexistent"}), &test_ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("code-review"));
        assert!(err.to_string().contains("safety"));
    }

    #[tokio::test]
    async fn skill_tool_missing_param() {
        let tool = SkillTool::new(test_registry());
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }
}
