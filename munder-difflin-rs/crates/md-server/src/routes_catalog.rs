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

fn collect_templates_from(dir: &std::path::Path, built_in: bool, out: &mut Vec<Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
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
                "builtIn": built_in,
                "parts": parts,
            }));
        }
    }
}

async fn templates_pro_list(State(state): State<AppState>) -> Json<Value> {
    let mut out = Vec::new();
    let user_dir = crate::routes_agent_file::user_templates_dir(&state);
    collect_templates_from(&user_dir, false, &mut out);
    if let Some(root) = resources_root() {
        collect_templates_from(&root.join("agent-templates"), true, &mut out);
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(Value::Array(out))
}

async fn templates_pro_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Json(json!({ "ok": false, "error": "invalid template id" }));
    }
    let user_dir = crate::routes_agent_file::user_templates_dir(&state);
    let mut dirs = vec![user_dir];
    if let Some(root) = resources_root() {
        dirs.push(root.join("agent-templates"));
    }
    for dir in dirs {
        for p in [
            dir.join(format!("{id}.agent.json")),
            dir.join(format!("{id}.json")),
            dir.join(&id),
        ] {
            if p.is_file()
                && let Ok(raw) = std::fs::read_to_string(&p)
                && let Ok(val) = serde_json::from_str::<Value>(&raw)
            {
                return Json(json!({ "ok": true, "file": val }));
            }
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

        // Scan MCP servers from ~/.claude/settings.json, ~/.gemini/settings.json,
        // ~/.codex/settings.json, and ~/.config/opencode/opencode.json
        for (engine, settings_path, key) in [
            ("claude", claude_dir.join("settings.json"), "mcpServers"),
            ("gemini", home_dir.join(".gemini").join("settings.json"), "mcpServers"),
            ("codex", home_dir.join(".codex").join("settings.json"), "mcpServers"),
            (
                "opencode",
                home_dir.join(".config").join("opencode").join("opencode.json"),
                "mcp",
            ),
            (
                "opencode",
                home_dir.join(".opencode").join("opencode.json"),
                "mcp",
            ),
        ] {
            if let Ok(raw) = std::fs::read_to_string(&settings_path)
                && let Ok(val) = serde_json::from_str::<Value>(&raw)
                && let Some(mcp) = val.get(key).and_then(|m| m.as_object())
            {
                for (name, cfg) in mcp {
                    let transport = if cfg.get("url").is_some()
                        || cfg.get("httpUrl").is_some()
                        || cfg.get("type").and_then(|t| t.as_str()) == Some("remote")
                    {
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

    let mut tools = Vec::new();
    for (name, bin) in [
        ("claude", "claude"),
        ("codex", "codex"),
        ("gemini", "gemini"),
        ("opencode", "opencode"),
        ("antigravity", "antigravity"),
        ("qwen", "qwen"),
        ("cursor", "agent"),
        ("git", "git"),
        ("gh", "gh"),
        ("node", "node"),
        ("python3", "python3"),
        ("cargo", "cargo"),
        ("uv", "uv"),
        ("graphify", "graphify"),
    ] {
        if let Some(p) = md_pty::manager::resolve_command(bin)
            && p.is_file()
        {
            tools.push(json!({
                "key": format!("tool:{name}"),
                "name": name,
                "bin": bin,
                "path": p.to_string_lossy().into_owned(),
            }));
        }
    }

    Json(json!({
        "plugins": plugins,
        "servers": servers,
        "tools": tools,
    }))
}

async fn missions_list(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.lock().unwrap().read();
    let list = cfg
        .extra
        .get("missions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Json(Value::Array(list))
}

async fn missions_save(
    State(state): State<AppState>,
    Json(missions): Json<Vec<Value>>,
) -> Json<Value> {
    match state
        .config
        .lock()
        .unwrap()
        .update(json!({ "missions": missions }))
    {
        Ok(_) => {
            state.emit("missions:updated", json!({ "ok": true }));
            Json(json!({ "ok": true }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

fn default_context_trigger() -> Value {
    json!({
        "compact": {
            "enabled": true,
            "everyMs": 2_400_000u64,
            "minContextPct": 50u64,
            "minContextPctLargeWindow": 30u64,
            "message": "Keep the current task, recent decisions, open questions, and file paths in play. Drop resolved tangents.",
            "force": false
        },
        "clear": {
            "enabled": false,
            "everyMs": 7_200_000u64,
            "minContextPct": 80u64,
            "minContextPctLargeWindow": 50u64,
            "message": ""
        },
        "switchClear": {
            "enabled": false,
            "minContextPct": 50u64,
            "minContextPctLargeWindow": 30u64
        }
    })
}

pub(crate) fn merged_context_trigger(raw: Option<&Value>) -> Value {
    let mut base = default_context_trigger();
    if let (Some(base_obj), Some(ovr_obj)) = (base.as_object_mut(), raw.and_then(|v| v.as_object()))
    {
        for (k, v) in ovr_obj {
            if let (Some(b_sub), Some(o_sub)) =
                (base_obj.get_mut(k).and_then(|x| x.as_object_mut()), v.as_object())
            {
                for (sk, sv) in o_sub {
                    b_sub.insert(sk.clone(), sv.clone());
                }
            } else {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
    base
}

async fn context_trigger_get(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.lock().unwrap().read();
    Json(merged_context_trigger(cfg.extra.get("contextTrigger")))
}

async fn context_trigger_set(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    for section in ["compact", "clear"] {
        if let Some(msg) = body
            .get(section)
            .and_then(|s| s.get("message"))
            .and_then(|m| m.as_str())
            && msg.chars().count() > 4000
        {
            return Json(json!({
                "ok": false,
                "error": "not saved: the message is longer than 4000 characters"
            }));
        }
    }
    let stored = {
        let cfg = state.config.lock().unwrap().read();
        cfg.extra.get("contextTrigger").cloned()
    };
    let next = if let Some(mut base) = stored.and_then(|v| v.as_object().cloned()) {
        if let Some(inc) = body.as_object() {
            for (k, v) in inc {
                base.insert(k.clone(), v.clone());
            }
        }
        merged_context_trigger(Some(&Value::Object(base)))
    } else {
        merged_context_trigger(Some(&body))
    };
    match state
        .config
        .lock()
        .unwrap()
        .update(json!({ "contextTrigger": next }))
    {
        Ok(cfg) => {
            let effective = merged_context_trigger(cfg.extra.get("contextTrigger"));
            state.emit("context:triggerUpdated", effective.clone());
            Json(json!({ "ok": true, "contextTrigger": effective }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentSettingPatchReq {
    kind: String,
    agent_id: Option<String>,
    key: Option<String>,
    value: Value,
}

async fn agent_settings_set(
    State(state): State<AppState>,
    Json(req): Json<AgentSettingPatchReq>,
) -> Json<Value> {
    let mut store = state.config.lock().unwrap();
    let cfg = store.read();
    let aid = req.agent_id.as_deref().unwrap_or("");
    let patch = match req.kind.as_str() {
        "mcp" => {
            let mut map = cfg
                .extra
                .get("agentMcp")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let mut row = map
                .get(aid)
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            if let Some(k) = req.key.as_deref() {
                if req.value.is_null() {
                    row.remove(k);
                } else {
                    row.insert(k.to_string(), req.value);
                }
            }
            map.insert(aid.to_string(), Value::Object(row));
            json!({ "agentMcp": map })
        }
        "skillOff" => {
            let mut map = cfg
                .extra
                .get("agentSkillsOff")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let mut arr: Vec<String> = map
                .get(aid)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(ToString::to_string)).collect())
                .unwrap_or_default();
            if let Some(folder) = req.key {
                let off = req.value.as_bool().unwrap_or(false);
                arr.retain(|x| x != &folder);
                if off {
                    arr.push(folder);
                }
            }
            map.insert(aid.to_string(), json!(arr));
            json!({ "agentSkillsOff": map })
        }
        "codexMemory" => {
            let mut arr: Vec<String> = cfg
                .extra
                .get("agentCodexMemoryOn")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(ToString::to_string)).collect())
                .unwrap_or_default();
            let on = req.value.as_bool().unwrap_or(false);
            arr.retain(|x| x != aid);
            if on && !aid.is_empty() {
                arr.push(aid.to_string());
            }
            json!({ "agentCodexMemoryOn": arr })
        }
        "browser" => {
            let mut map = cfg
                .extra
                .get("agentBrowser")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            map.insert(aid.to_string(), req.value);
            json!({ "agentBrowser": map })
        }
        _ => return Json(json!({ "ok": false, "error": "unknown setting kind" })),
    };

    match store.update(patch) {
        Ok(next) => {
            let val = serde_json::to_value(&next).unwrap_or(Value::Null);
            state.emit("config:changed", val.clone());
            Json(val)
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillInstallReq {
    url: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillUninstallReq {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityInstallReq {
    entry_id: String,
}

async fn skills_install(
    State(state): State<AppState>,
    Json(req): Json<SkillInstallReq>,
) -> Json<Value> {
    let mut store = state.config.lock().unwrap();
    let cfg = store.read();
    let mut installed = cfg
        .extra
        .get("installedSkills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let name = req.name.unwrap_or_else(|| req.url.clone());
    if !installed
        .iter()
        .any(|x| x.get("name").and_then(|v| v.as_str()) == Some(&name))
    {
        installed.push(json!({
            "name": name,
            "url": req.url,
            "path": format!("skills/{name}"),
            "scope": "user",
        }));
    }
    match store.update(json!({ "installedSkills": installed })) {
        Ok(_) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn skills_uninstall(
    State(state): State<AppState>,
    Json(req): Json<SkillUninstallReq>,
) -> Json<Value> {
    let mut store = state.config.lock().unwrap();
    let cfg = store.read();
    let mut installed = cfg
        .extra
        .get("installedSkills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    installed.retain(|x| x.get("path").and_then(|v| v.as_str()) != Some(&req.path));
    match store.update(json!({ "installedSkills": installed })) {
        Ok(_) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn capabilities_install(
    State(state): State<AppState>,
    Json(req): Json<CapabilityInstallReq>,
) -> Json<Value> {
    let steps: Vec<(String, Vec<String>)> = resources_root()
        .and_then(|root| std::fs::read_to_string(root.join("capabilities").join("catalog.json")).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|cat| {
            cat.get("entries")?
                .as_array()?
                .iter()
                .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(&req.entry_id))
                .and_then(|e| e.get("install"))
                .and_then(|ins| ins.get("steps"))
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|step| {
                            let label = step
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Install step")
                                .to_string();
                            let argv: Vec<String> = step
                                .get("argv")?
                                .as_array()?
                                .iter()
                                .filter_map(|a| a.as_str().map(ToString::to_string))
                                .collect();
                            if argv.is_empty() {
                                None
                            } else {
                                Some((label, argv))
                            }
                        })
                        .collect()
                })
        })
        .unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    let emit_line = |line: String, out: &mut Vec<String>| {
        out.push(line.clone());
        state.emit(
            "capabilities:installLine",
            json!({ "entryId": req.entry_id, "line": line }),
        );
    };

    let path_env = crate::hive_bootstrap::ensure_extra_path_entries(std::env::var("PATH").ok());
    for (label, argv) in &steps {
        let bin = &argv[0];
        let args = &argv[1..];
        emit_line(format!("$ {} ({label})", argv.join(" ")), &mut lines);
        let Some(resolved) = md_pty::manager::resolve_command(bin) else {
            let msg = format!("Command not found on PATH: {bin}");
            emit_line(msg.clone(), &mut lines);
            return Json(json!({ "ok": false, "lines": lines, "error": msg }));
        };
        let out = tokio::process::Command::new(resolved)
            .args(args)
            .env("PATH", &path_env)
            .output()
            .await;
        match out {
            Ok(o) => {
                for chunk in [
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr),
                ] {
                    for l in chunk.lines() {
                        let trimmed = l.trim_end();
                        if !trimmed.is_empty() {
                            emit_line(trimmed.to_string(), &mut lines);
                        }
                    }
                }
                if !o.status.success() {
                    let msg = format!("{bin} exited with status {}", o.status);
                    emit_line(msg.clone(), &mut lines);
                    return Json(json!({ "ok": false, "lines": lines, "error": msg }));
                }
            }
            Err(e) => {
                let msg = format!("Failed to run {bin}: {e}");
                emit_line(msg.clone(), &mut lines);
                return Json(json!({ "ok": false, "lines": lines, "error": msg }));
            }
        }
    }

    if lines.is_empty() {
        emit_line("Registered in local configuration".to_string(), &mut lines);
    }

    let mut store = state.config.lock().unwrap();
    let cfg = store.read();
    let mut caps = cfg
        .extra
        .get("installedCapabilities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if !caps.iter().any(|v| v.as_str() == Some(&req.entry_id)) {
        caps.push(json!(req.entry_id));
    }
    match store.update(json!({ "installedCapabilities": caps })) {
        Ok(_) => Json(json!({ "ok": true, "lines": lines })),
        Err(e) => Json(json!({ "ok": false, "lines": lines, "error": e.to_string() })),
    }
}

async fn office_themes_list(State(state): State<AppState>) -> Json<Value> {
    let dir = state.data_dir.join("office-themes");
    let mut themes = Vec::new();
    let bad: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let id = entry.file_name().to_string_lossy().into_owned();
                let label = std::fs::read_to_string(p.join("theme.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .and_then(|v| v.get("label").and_then(|l| l.as_str()).map(ToString::to_string))
                    .unwrap_or_else(|| id.clone());
                themes.push(json!({ "id": id, "label": label }));
            }
        }
    }
    Json(json!({ "themes": themes, "bad": bad }))
}

async fn office_themes_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let p = state.data_dir.join("office-themes").join(&id).join("theme.json");
    match std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(v) => Json(v),
        None => Json(Value::Null),
    }
}

async fn code_themes_list(State(state): State<AppState>) -> Json<Value> {
    let dir = state.data_dir.join("code-themes");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            if let Ok(raw) = std::fs::read_to_string(entry.path())
                && let Ok(val) = serde_json::from_str::<Value>(&raw)
            {
                out.push(val);
            }
        }
    }
    Json(Value::Array(out))
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
        .route("/api/v1/capabilities/install", post(capabilities_install))
        .route("/api/v1/presets", get(presets_list))
        .route("/api/v1/skills", get(skills_list))
        .route("/api/v1/skills/install", post(skills_install))
        .route("/api/v1/skills/uninstall", post(skills_uninstall))
        .route(
            "/api/v1/themes/workbench",
            get(theme_workbench_get).post(theme_workbench_set),
        )
        .route("/api/v1/themes/office", get(office_themes_list))
        .route("/api/v1/themes/office/{id}", get(office_themes_read))
        .route("/api/v1/themes/code", get(code_themes_list))
        .route("/api/v1/agents/export", post(agent_export))
        .route("/api/v1/agents/import", post(agent_import))
        .route("/api/v1/agents/settings", post(agent_settings_set))
        .route("/api/v1/missions", get(missions_list).post(missions_save))
        .route(
            "/api/v1/context-trigger",
            get(context_trigger_get).post(context_trigger_set),
        )
}
