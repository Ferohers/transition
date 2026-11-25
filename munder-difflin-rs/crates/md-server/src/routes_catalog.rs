//! Phase 7 routes: bundled agent templates, simulation presets, skills,
//! theme workbench, and `.agent.json` import/export.

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, State},
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
    let cwd_res = PathBuf::from("./resources");
    if cwd_res.is_dir() {
        return Some(cwd_res);
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1).take(4) {
            let p = ancestor.join("resources");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let manifest_res = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("resources");
    if manifest_res.is_dir() {
        return Some(manifest_res);
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

async fn templates_pro_list() -> Json<Value> {
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
        let id = file
            .strip_suffix(".agent.json")
            .or_else(|| file.strip_suffix(".json"))
            .unwrap_or(&file)
            .to_string();
        if let Ok(raw) = std::fs::read_to_string(&p)
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
        {
            let name = val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let description = val
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let role = val
                .get("configs")
                .and_then(|c| c.get("role"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let parts = val
                .get("parts")
                .cloned()
                .unwrap_or_else(|| json!([]));
            out.push(json!({
                "id": id,
                "name": name,
                "description": description,
                "role": role,
                "builtIn": true,
                "parts": parts,
            }));
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(Value::Array(out))
}

async fn templates_pro_read(Path(id): Path<String>) -> Json<Value> {
    let Some(root) = resources_root() else {
        return Json(json!({ "ok": false, "error": "resources directory not found" }));
    };
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Json(json!({ "ok": false, "error": "invalid template id" }));
    }
    let dir = root.join("agent-templates");
    let candidates = [
        dir.join(format!("{id}.agent.json")),
        dir.join(format!("{id}.json")),
        dir.join(&id),
    ];
    for p in candidates {
        if p.is_file()
            && let Ok(raw) = std::fs::read_to_string(&p)
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
        {
            return Json(json!({ "ok": true, "file": val }));
        }
    }
    Json(json!({ "ok": false, "error": "template not found" }))
}

async fn capabilities_catalog() -> Json<Value> {
    if let Some(root) = resources_root() {
        let p = root.join("capabilities").join("catalog.json");
        if let Ok(raw) = std::fs::read_to_string(&p)
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
        {
            return Json(val);
        }
    }
    Json(json!({ "version": 1, "entries": [], "purposes": [] }))
}

async fn capabilities_installed() -> Json<Value> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let mut plugins = Vec::new();
    let mut servers = Vec::new();

    if let Some(ref home_dir) = home {
        let claude_dir = home_dir.join(".claude");
        let enabled_map: std::collections::HashMap<String, bool> =
            std::fs::read_to_string(claude_dir.join("settings.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v.get("enabledPlugins").and_then(|e| e.as_object()).cloned())
                .map(|obj| {
                    obj.into_iter()
                        .filter_map(|(k, v)| v.as_bool().map(|b| (k, b)))
                        .collect()
                })
                .unwrap_or_default();

        if let Ok(raw) = std::fs::read_to_string(claude_dir.join("plugins").join("installed_plugins.json"))
            && let Ok(val) = serde_json::from_str::<Value>(&raw)
            && let Some(plug_obj) = val.get("plugins").and_then(|p| p.as_object())
        {
            for (plugin_id, installs) in plug_obj {
                let Some(arr) = installs.as_array() else {
                    continue;
                };
                for inst in arr {
                    let scope = inst
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user");
                    let where_path = inst
                        .get("installPath")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let (name, marketplace) = match plugin_id.split_once('@') {
                        Some((n, m)) => (n, Some(m)),
                        None => (plugin_id.as_str(), None),
                    };
                    let enabled = enabled_map.get(plugin_id).copied();
                    let installed_at = inst
                        .get("installedAt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.get(..10));
                    let mut entry = json!({
                        "key": format!("claude:{plugin_id}:{scope}"),
                        "id": plugin_id,
                        "name": name,
                        "engine": "claude",
                        "scope": scope,
                        "where": where_path,
                        "skills": [],
                        "servers": [],
                    });
                    if let Some(m) = marketplace {
                        entry["marketplace"] = json!(m);
                    }
                    if let Some(b) = enabled {
                        entry["enabled"] = json!(b);
                    }
                    if let Some(v) = inst.get("version").and_then(|v| v.as_str()) {
                        entry["version"] = json!(v);
                    }
                    if let Some(d) = installed_at {
                        entry["installedAt"] = json!(d);
                    }
                    plugins.push(entry);
                }
            }
        }

        // Scan MCP servers from ~/.claude/settings.json and ~/.gemini/settings.json
        for (engine, settings_path) in [
            ("claude", claude_dir.join("settings.json")),
            ("gemini", home_dir.join(".gemini").join("settings.json")),
        ] {
            if let Ok(raw) = std::fs::read_to_string(&settings_path)
                && let Ok(val) = serde_json::from_str::<Value>(&raw)
                && let Some(mcp) = val.get("mcpServers").and_then(|m| m.as_object())
            {
                for (name, cfg) in mcp {
                    let transport = if cfg.get("url").is_some() || cfg.get("httpUrl").is_some() {
                        "http"
                    } else {
                        "stdio"
                    };
                    servers.push(json!({
                        "key": format!("{engine}:mcp:{name}"),
                        "name": name,
                        "engine": engine,
                        "scope": "user",
                        "transport": transport,
                        "where": settings_path.display().to_string(),
                    }));
                }
            }
        }
    }

    Json(json!({
        "plugins": plugins,
        "servers": servers,
        "tools": [],
    }))
}

pub fn catalog_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/templates", get(templates_list))
        .route("/api/v1/templates/pro", get(templates_pro_list))
        .route("/api/v1/templates/pro/{id}", get(templates_pro_read))
        .route("/api/v1/catalog/capabilities", get(capabilities_catalog))
        .route(
            "/api/v1/catalog/capabilities/installed",
            post(capabilities_installed),
        )
        .route("/api/v1/presets", get(presets_list))
        .route("/api/v1/skills", get(skills_list))
        .route(
            "/api/v1/themes/workbench",
            get(theme_workbench_get).post(theme_workbench_set),
        )
        .route("/api/v1/agents/export", post(agent_export))
        .route("/api/v1/agents/import", post(agent_import))
}
