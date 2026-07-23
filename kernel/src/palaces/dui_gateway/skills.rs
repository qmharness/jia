use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;

/// Resolve the skills/ directory. CARGO_MANIFEST_DIR for the kernel crate
/// is `kernel/`, so we go up one level to the project root.
fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("skills"))
        .unwrap_or_else(|| PathBuf::from("skills"))
}
use axum::extract::State;

use crate::palaces::li_skill::SkillRegistry;
use crate::palaces::li_skill::loader::SkillLoader;

use super::AppState;

/// Safe preview that slices at a valid UTF-8 char boundary.
pub fn prompt_preview(prompt: &str, max_chars: usize) -> &str {
    if prompt.len() <= max_chars {
        return prompt;
    }
    let mut end = max_chars;
    while end > 0 && !prompt.is_char_boundary(end) {
        end -= 1;
    }
    &prompt[..end]
}

pub async fn handle_skills(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"skills": []})),
    };
    let reg = earth.skills.read().unwrap_or_else(|e| e.into_inner());
    let skills = reg.list_all_with_status();
    tracing::info!("Skills API: {} skills loaded", skills.len());
    let list: Vec<_> = skills
        .iter()
        .map(|(s, disabled)| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "source_path": s.source_path.to_string_lossy(),
                "prompt": prompt_preview(&s.prompt, 200),
                "auto_evolve": s.auto_evolve,
                "evolve_min_confidence": s.evolve_min_confidence,
                "evolve_max_revisions_per_session": s.evolve_max_revisions_per_session,
                "evolve_reflection_threshold": s.evolve_reflection_threshold,
                "always": s.always,
                "has_paths": s.paths.is_some(),
                "disabled": disabled,
            })
        })
        .collect();
    Json(serde_json::json!({"skills": list}))
}

pub async fn handle_skills_evolution(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    let store = &earth.store;

    // Recent revision diffs (last 20)
    let recent_revisions = store.load_recent_revisions(20).unwrap_or_default();

    // Reflection summaries for skills with any reflections
    let skill_names: Vec<String> = {
        let reg = earth.skills.read().unwrap_or_else(|e| e.into_inner());
        reg.list_all().iter().map(|s| s.name.clone()).collect()
    };
    let mut summaries = Vec::new();
    for name in &skill_names {
        if let Ok(s) = store.load_reflection_summary(name)
            && s["total_reflections"].as_i64().unwrap_or(0) > 0
        {
            summaries.push(s);
        }
    }

    // Aggregate confidence trend: average confidence per revision (chronological)
    let confidence_trend: Vec<f64> = recent_revisions
        .iter()
        .rev() // oldest first for trend
        .filter_map(|r| r["avg_confidence"].as_f64())
        .collect();

    Json(serde_json::json!({
        "recent_revisions": recent_revisions,
        "reflection_summaries": summaries,
        "confidence_trend": confidence_trend,
        "total_revisions": store.count_total_revisions().unwrap_or(0),
    }))
}

pub async fn handle_skills_reload(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    // SkillLoader::load_directory_sync 是同步文件 IO,移入 spawn_blocking
    // 避免阻塞 tokio runtime;skills 的 RwLock 守卫只留在 async 侧,
    // 不跨入 spawn_blocking 边界。
    let dir = skills_dir();
    let (mut new_reg, result) = match tokio::task::spawn_blocking(move || {
        let mut reg = SkillRegistry::new();
        let result = SkillLoader::load_directory_sync(&dir, &mut reg);
        (reg, result)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Skills reload task failed: {e}");
            return Json(serde_json::json!({"loaded": 0}));
        }
    };
    let loaded = match result {
        Ok(n) => n,
        Err(e) => {
            // S3: 加载失败时保留现有 registry——此前无条件用(可能为空的)
            // new_reg 整体替换,目录读取失败即清空全部 skills。
            tracing::warn!("Skills reload failed: {e}");
            return Json(serde_json::json!({"error": format!("reload failed: {e}"), "loaded": 0}));
        }
    };
    // Preserve disabled state across reload
    let old_disabled = {
        let old = earth.skills.read().unwrap_or_else(|e| e.into_inner());
        old.disabled_names().into_iter().cloned().collect()
    };
    new_reg.set_disabled(&old_disabled);
    *earth.skills.write().unwrap_or_else(|e| e.into_inner()) = new_reg;
    Json(serde_json::json!({"loaded": loaded}))
}

pub async fn handle_skills_toggle(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return Json(serde_json::json!({"error": "Missing 'name' field"})),
    };
    let disabled = body
        .get("disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut reg = earth.skills.write().unwrap_or_else(|e| e.into_inner());
    if disabled {
        if reg.disable(name) {
            Json(serde_json::json!({"ok": true, "name": name, "disabled": true}))
        } else {
            Json(serde_json::json!({"error": format!("Skill '{}' not found", name)}))
        }
    } else {
        if reg.enable(name) {
            Json(serde_json::json!({"ok": true, "name": name, "disabled": false}))
        } else {
            Json(serde_json::json!({"error": format!("Skill '{}' was not disabled", name)}))
        }
    }
}

pub async fn handle_skills_evolve_toggle(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return Json(serde_json::json!({"error": "Missing 'name' field"})),
    };
    let enable = body
        .get("auto_evolve")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = {
        let reg = earth.skills.read().unwrap_or_else(|e| e.into_inner());
        match reg
            .list_all_with_status()
            .iter()
            .find(|(s, _)| s.name == name)
        {
            Some((s, _)) => s.source_path.clone(),
            None => {
                return Json(serde_json::json!({"error": format!("Skill '{}' not found", name)}));
            }
        }
    };

    // Toggle auto_evolve in SKILL.md frontmatter
    match toggle_frontmatter_auto_evolve(&path, enable).await {
        Ok(_) => {
            // Reload to pick up the change(同步文件 IO 移入 spawn_blocking,
            // 同 handle_skills_reload;RwLock 守卫不跨入阻塞边界)
            let dir = skills_dir();
            let reload = tokio::task::spawn_blocking(move || {
                let mut reg = SkillRegistry::new();
                let _ = SkillLoader::load_directory_sync(&dir, &mut reg);
                reg
            })
            .await;
            match reload {
                Ok(mut new_reg) => {
                    let old_disabled = {
                        let old = earth.skills.read().unwrap_or_else(|e| e.into_inner());
                        old.disabled_names().into_iter().cloned().collect()
                    };
                    new_reg.set_disabled(&old_disabled);
                    *earth.skills.write().unwrap_or_else(|e| e.into_inner()) = new_reg;
                    Json(serde_json::json!({"ok": true, "name": name, "auto_evolve": enable}))
                }
                Err(e) => Json(serde_json::json!({"error": format!("Skills reload failed: {e}")})),
            }
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

pub async fn toggle_frontmatter_auto_evolve(
    path: &std::path::Path,
    enable: bool,
) -> Result<(), String> {
    let path = path.to_path_buf();
    let path_display = path.display().to_string();
    let read_path = path.clone();
    let content = tokio::task::spawn_blocking(move || std::fs::read_to_string(&read_path))
        .await
        .map_err(|_| "Internal error".to_string())?
        .map_err(|e| format!("Cannot read {}: {e}", path_display))?;

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || lines[0] != "---" {
        return Err("No frontmatter found".into());
    }
    let end = lines[1..].iter().position(|l| *l == "---").map(|p| p + 1);
    let end = match end {
        Some(e) => e,
        None => return Err("Unclosed frontmatter".into()),
    };

    let key = "auto_evolve:";
    let new_val = if enable { "true" } else { "false" };
    let existing = lines[1..end]
        .iter()
        .position(|l| l.trim_start().starts_with(key));

    let mut result = String::new();
    let mut replaced = false;
    for (j, line) in lines.iter().enumerate() {
        if j > 0 && j < end && existing == Some(j - 1) {
            let indent = line.len() - line.trim_start().len();
            result.push_str(&format!("{:indent$}{key} {new_val}\n", "", indent = indent));
            replaced = true;
        } else if j == end && !replaced {
            result.push_str(&format!("{key} {new_val}\n"));
            result.push_str(line);
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    let final_content = result.trim_end().to_string();
    let write_path_display = path.display().to_string();
    tokio::task::spawn_blocking(move || std::fs::write(&path, final_content))
        .await
        .map_err(|_| "Internal error".to_string())?
        .map_err(|e| format!("Cannot write {}: {e}", write_path_display))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_preview_truncates() {
        let long = "a".repeat(500);
        let preview = prompt_preview(&long, 100);
        assert!(preview.len() <= 100);
    }

    #[test]
    fn prompt_preview_passthrough() {
        assert_eq!(prompt_preview("hello", 100), "hello");
    }
}
