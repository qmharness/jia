use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;

use crate::palaces::zhen_tool::builtin::cron::TriggerMode;

use super::AppState;

// ── Cron ────────────────────────────────────────────────

pub async fn handle_cron_list(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"jobs": []})),
    };
    let jobs = earth.cron.list().unwrap_or_default();
    let list: Vec<_> = jobs
        .iter()
        .map(|j| {
            serde_json::json!({
                "name": j.name,
                "schedule": j.schedule,
                "prompt": j.prompt,
                "enabled": j.enabled,
                "last_fired_at": j.last_fired_at,
                "last_response": j.last_response,
                "cooldown_secs": j.cooldown_secs,
                "trigger": j.trigger,
            })
        })
        .collect();
    Json(serde_json::json!({"jobs": list}))
}

#[derive(Debug, Deserialize)]
pub struct CronManageBody {
    action: String,
    name: Option<String>,
    schedule: Option<String>,
    prompt: Option<String>,
    cooldown_secs: Option<u64>,
    trigger: Option<String>,
}

pub async fn handle_cron_manage(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<CronManageBody>,
) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    let result = match body.action.as_str() {
        "add" => {
            let name = body.name.as_deref().unwrap_or("");
            let schedule = body.schedule.as_deref().unwrap_or("");
            let prompt = body.prompt.as_deref().unwrap_or("");
            if name.is_empty() || schedule.is_empty() {
                return Json(serde_json::json!({"error": "name and schedule are required"}));
            }
            let trigger = match body.trigger.as_deref() {
                Some("once") => TriggerMode::Once,
                _ => TriggerMode::Schedule,
            };
            earth
                .cron
                .add(name, schedule, prompt, body.cooldown_secs, trigger)
                .map(|_| serde_json::json!({"added": name}))
        }
        "remove" => {
            let name = body.name.as_deref().unwrap_or("");
            earth
                .cron
                .remove(name)
                .map(|_| serde_json::json!({"removed": name}))
        }
        "update" => {
            let name = body.name.as_deref().unwrap_or("");
            if name.is_empty() {
                return Json(serde_json::json!({"error": "name is required"}));
            }
            earth
                .cron
                .update(
                    name,
                    body.schedule.as_deref(),
                    body.prompt.as_deref(),
                    body.cooldown_secs,
                )
                .map(|_| serde_json::json!({"updated": name}))
        }
        "enable" | "disable" => {
            let name = body.name.as_deref().unwrap_or("");
            let enabled = body.action == "enable";
            earth
                .cron
                .set_enabled(name, enabled)
                .map(|_| serde_json::json!({"updated": name, "enabled": enabled}))
        }
        _ => Err(format!("Unknown action: {}", body.action)),
    };
    match result {
        Ok(j) => Json(j),
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_manage_body_deserializes() {
        let b: CronManageBody = serde_json::from_str(
            r#"{"action": "create", "name": "test", "schedule": "* * * * *"}"#,
        )
        .unwrap();
        assert_eq!(b.action, "create");
        assert_eq!(b.name.unwrap(), "test");
    }
}
