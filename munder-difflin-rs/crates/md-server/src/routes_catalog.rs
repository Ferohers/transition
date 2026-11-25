//! Phase 7 routes: bundled agent templates, simulation presets, skills,
//! theme workbench, and `.agent.json` import/export.

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

fn resources_root() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("MD_RESOURCES_DIR") {
        let p = PathBuf::from(explicit.trim());
        if p.is_dir() {
            return Some(p);
        }
    }
    for candidate in [
        "/Users/altan/Documents/transition/Munder Difflin.app/Contents/Resources",
        "./resources",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn list_dir_names(subdir: &str, only_dirs: bool) -> Vec<String> {
    let Some(root) = resources_root() else {
        return Vec::new();
    };
    let dir = root.join(subdir);
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if only_dirs && !p.is_dir() {
                return None;
            }
            if !only_dirs && !p.is_file() {
                return None;
            }
            Some(e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
}

async fn templates_list() -> Json<Value> {
    let Some(root) = resources_root() else {
        return Json(json!([]));
    };
    let dir = root.join("agent-templates");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Json(json!([]));
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let file = entry.file_name().to_string_lossy().into_owned();
        if let Ok(raw) = std::fs::read_to_string(&p)
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
        {
            out.push(json!({ "file": file, "template": val }));
        }
    }
    out.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    Json(Value::Array(out))
}

async fn presets_list() -> Json<Vec<String>> {
    Json(list_dir_names("simulation-presets", true))
}

async fn skills_list() -> Json<Vec<String>> {
    Json(list_dir_names("skills", true))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeUpdateReq {
    office_theme: Option<String>,
    theme: Option<String>,
    skin: Option<String>,
}

async fn theme_workbench_get(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.lock().unwrap().read();
    Json(json!({
        "theme": cfg.theme,
        "skin": cfg.skin,
        "officeTheme": cfg.office_theme.unwrap_or_else(|| "executive-wing".to_string()),
        "presets": list_dir_names("simulation-presets", true),
    }))
}

async fn theme_workbench_set(
    State(state): State<AppState>,
    Json(req): Json<ThemeUpdateReq>,
) -> Json<Value> {
    let mut patch = serde_json::Map::new();
    if let Some(t) = req.theme {
        patch.insert("theme".to_string(), json!(t));
    }
    if let Some(s) = req.skin {
        patch.insert("skin".to_string(), json!(s));
    }
    if let Some(o) = req.office_theme {
        patch.insert("officeTheme".to_string(), json!(o));
    }
    match state.config.lock().unwrap().update(Value::Object(patch)) {
        Ok(cfg) => {
            if let Ok(val) = serde_json::to_value(&cfg) {
                state.emit("config:changed", val);
            }
            Json(json!({ "ok": true, "config": cfg }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportAgentReq {
    id: String,
    name: String,
    role: Option<String>,
    engine: Option<String>,
}

async fn agent_export(Json(req): Json<ExportAgentReq>) -> Json<Value> {
    Json(json!({
        "format": "munder-difflin.agent.v1",
        "agent": {
            "id": req.id,
            "name": req.name,
            "role": req.role.unwrap_or_else(|| "engineer".to_string()),
            "engine": req.engine.unwrap_or_else(|| "claude".to_string())
        }
    }))
}

async fn agent_import(Json(bundle): Json<Value>) -> Json<Value> {
    let valid = bundle.get("agent").and_then(|a| a.get("name")).is_some();
    if !valid {
        return Json(json!({ "ok": false, "error": "missing agent.name in .agent.json" }));
    }
    Json(json!({ "ok": true, "agent": bundle["agent"] }))
}

pub fn catalog_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/templates", get(templates_list))
        .route("/api/v1/presets", get(presets_list))
        .route("/api/v1/skills", get(skills_list))
        .route(
            "/api/v1/themes/workbench",
            get(theme_workbench_get).post(theme_workbench_set),
        )
        .route("/api/v1/agents/export", post(agent_export))
        .route("/api/v1/agents/import", post(agent_import))
}
