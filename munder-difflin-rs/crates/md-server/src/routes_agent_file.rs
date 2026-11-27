//! HTTP routes for `.agent.json` planning, applying, exporting, skill choices,
//! and custom user template management.

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{delete, get, post},
};
use md_core::repo_fs::is_safe_id;
use md_hive::memory;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFilePlanReq {
    agent_id: Option<String>,
    file: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFileApplyReq {
    agent_id: String,
    file: Value,
    #[serde(default)]
    accept: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    fresh: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFileExportReq {
    agent_id: String,
    #[serde(default)]
    parts: Vec<String>,
    #[serde(default)]
    extra_skill_paths: Option<Vec<String>>,
    description: Option<String>,
    author: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemplateSaveReq {
    name: String,
    description: Option<String>,
    file: Value,
    #[serde(default)]
    accepted: Vec<String>,
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "custom-agent".to_string()
    } else {
        trimmed
    }
}

fn file_parts(file: &Value) -> HashSet<String> {
    if let Some(arr) = file.get("parts").and_then(|v| v.as_array()) {
        let set: HashSet<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect();
        if !set.is_empty() {
            return set;
        }
    }
    let mut set = HashSet::new();
    for key in [
        "configs",
        "mcpServers",
        "skills",
        "memories",
        "automations",
        "permissions",
        "cardNotes",
        "catalog",
    ] {
        if file.get(key).is_some_and(|v| !v.is_null()) {
            set.insert(key.to_string());
        }
    }
    set
}

fn find_agent_card(state: &AppState, agent_id: &str) -> Option<Map<String, Value>> {
    let home = state.harness_home();
    if let Some(snap) = state.roster.lock().unwrap().read(home.as_deref()) {
        for a in snap.agents.iter().chain(snap.archived.iter()) {
            if a.get("id").and_then(|v| v.as_str()) == Some(agent_id)
                && let Some(obj) = a.as_object()
            {
                return Some(obj.clone());
            }
        }
    }
    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    reg.get("agents")
        .and_then(|a| a.get(agent_id))
        .and_then(|v| v.as_object())
        .cloned()
}

#[allow(clippy::too_many_arguments)]
fn make_change(
    id: impl Into<String>,
    part: &str,
    label: impl Into<String>,
    before: Option<String>,
    after: Option<String>,
    detail: Option<String>,
    risk: &str,
    default_on: bool,
    next_start: bool,
) -> Value {
    json!({
        "id": id.into(),
        "part": part,
        "label": label.into(),
        "before": before,
        "after": after,
        "detail": detail,
        "risk": risk,
        "defaultOn": default_on,
        "nextStart": next_start,
        "missing": [],
    })
}

async fn agent_file_plan(
    State(state): State<AppState>,
    Json(req): Json<AgentFilePlanReq>,
) -> Json<Value> {
    let file = &req.file;
    if !file.is_object() {
        return Json(json!({ "ok": false, "error": "invalid agent file" }));
    }
    let parts = file_parts(file);
    let existing = req
        .agent_id
        .as_deref()
        .and_then(|id| find_agent_card(&state, id));
    let cfg = state.config.lock().unwrap().read();
    let agent_id_str = req.agent_id.as_deref().unwrap_or("");

    let mut changes = Vec::new();

    if parts.contains("configs")
        && let Some(c) = file.get("configs").and_then(|v| v.as_object())
    {
        if let Some(name) = file.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            let cur = existing
                .as_ref()
                .and_then(|a| a.get("name").and_then(|v| v.as_str()).map(ToString::to_string));
            if cur.as_deref() != Some(name) {
                changes.push(make_change(
                    "configs:name",
                    "configs",
                    "Name",
                    cur,
                    Some(name.to_string()),
                    None,
                    "none",
                    true,
                    false,
                ));
            }
        }
        if let Some(role) = c.get("role").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            let cur = existing.as_ref().and_then(|a| {
                a.get("description")
                    .or_else(|| a.get("role"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            });
            changes.push(make_change(
                "configs:role",
                "configs",
                "Role",
                cur,
                Some(role.to_string()),
                Some(role.to_string()),
                "none",
                true,
                false,
            ));
        }
        if let Some(goal) = c.get("goal").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            let cur = existing
                .as_ref()
                .and_then(|a| a.get("goal").and_then(|v| v.as_str()).map(ToString::to_string));
            changes.push(make_change(
                "configs:goal",
                "configs",
                "Goal",
                cur,
                Some(goal.to_string()),
                Some(goal.to_string()),
                "none",
                true,
                false,
            ));
        }
        if c.get("provider").is_some() || c.get("model").is_some() {
            let prov = c.get("provider").and_then(|v| v.as_str()).unwrap_or("");
            let model = c.get("model").and_then(|v| v.as_str()).unwrap_or("");
            let after = [prov, model]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            if !after.is_empty() {
                changes.push(make_change(
                    "configs:engine",
                    "configs",
                    "Engine & model",
                    None,
                    Some(after),
                    None,
                    "review",
                    true,
                    true,
                ));
            }
        }
        if let Some(tl) = c.get("tokenLimit").filter(|v| v.is_object()) {
            changes.push(make_change(
                "configs:tokenLimit",
                "configs",
                "Token limit",
                None,
                Some(tl.to_string()),
                None,
                "review",
                true,
                false,
            ));
        }
    }

    if parts.contains("mcpServers")
        && let Some(mcp) = file.get("mcpServers").and_then(|v| v.as_object())
    {
        let active_mcp = cfg
            .extra
            .get("agentMcp")
            .and_then(|m| m.get(agent_id_str))
            .and_then(|m| m.as_object());
        if let Some(cat) = mcp.get("catalog").and_then(|v| v.as_array()) {
            for item in cat {
                let Some(name) = item.as_str().filter(|s| !s.is_empty()) else {
                    continue;
                };
                let already = active_mcp
                    .and_then(|m| m.get(name))
                    .and_then(|v| {
                        v.as_bool()
                            .or_else(|| v.get("enabled").and_then(|b| b.as_bool()))
                    })
                    .unwrap_or(false);
                changes.push(make_change(
                    format!("mcp:catalog:{name}"),
                    "mcpServers",
                    name,
                    if already { Some("Enabled".to_string()) } else { None },
                    Some("Enable MCP server".to_string()),
                    Some(format!("Catalog MCP server: {name}")),
                    "review",
                    true,
                    false,
                ));
            }
        }
        if let Some(custom) = mcp.get("custom").and_then(|v| v.as_array()) {
            for (idx, item) in custom.iter().enumerate() {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("custom-mcp");
                changes.push(make_change(
                    format!("mcp:custom:{idx}:{name}"),
                    "mcpServers",
                    name,
                    None,
                    Some("Custom MCP server".to_string()),
                    Some(serde_json::to_string_pretty(item).unwrap_or_default()),
                    "runs-code",
                    false,
                    false,
                ));
            }
        }
    }

    if parts.contains("skills")
        && let Some(sk) = file.get("skills").and_then(|v| v.as_object())
    {
        let off_list: HashSet<String> = cfg
            .extra
            .get("agentSkillsOff")
            .and_then(|m| m.get(agent_id_str))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(cat) = sk.get("catalog").and_then(|v| v.as_array()) {
            for item in cat {
                let Some(name) = item.as_str().filter(|s| !s.is_empty()) else {
                    continue;
                };
                let before = if off_list.contains(name) {
                    Some("Disabled".to_string())
                } else {
                    None
                };
                changes.push(make_change(
                    format!("skill:catalog:{name}"),
                    "skills",
                    name,
                    before,
                    Some("Enable skill".to_string()),
                    Some(format!("Catalog skill: {name}")),
                    "review",
                    true,
                    false,
                ));
            }
        }
        if let Some(custom) = sk.get("custom").and_then(|v| v.as_array()) {
            for (idx, item) in custom.iter().enumerate() {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("custom-skill");
                let detail = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| serde_json::to_string_pretty(item).ok());
                changes.push(make_change(
                    format!("skill:custom:{idx}:{name}"),
                    "skills",
                    name,
                    None,
                    Some("Custom skill".to_string()),
                    detail,
                    "runs-code",
                    false,
                    false,
                ));
            }
        }
    }

    if parts.contains("memories")
        && let Some(mem_val) = file.get("memories")
    {
        let content = mem_val
            .as_str()
            .or_else(|| mem_val.get("content").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim();
        if !content.is_empty() {
            let before = if !agent_id_str.is_empty()
                && let Some(home) = state.harness_home()
            {
                memory::read_agent_memory(&home, agent_id_str)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            } else {
                None
            };
            changes.push(make_change(
                "memories:content",
                "memories",
                "Memory notes",
                before,
                Some(content.to_string()),
                Some(content.to_string()),
                "review",
                true,
                false,
            ));
        }
    }

    if parts.contains("automations")
        && let Some(autos) = file.get("automations").and_then(|v| v.as_array())
    {
        for (idx, item) in autos.iter().enumerate() {
            let label = item
                .get("label")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("Automation");
            changes.push(make_change(
                format!("auto:{idx}"),
                "automations",
                label,
                None,
                Some("Add scheduled mission".to_string()),
                Some(serde_json::to_string_pretty(item).unwrap_or_default()),
                "review",
                true,
                false,
            ));
        }
    }

    if parts.contains("permissions")
        && let Some(perms) = file.get("permissions").and_then(|v| v.as_object())
        && let Some(flags) = perms.get("flags").and_then(|v| v.as_array())
    {
        let joined: Vec<&str> = flags.iter().filter_map(|v| v.as_str()).collect();
        if !joined.is_empty() {
            changes.push(make_change(
                "permissions:flags",
                "permissions",
                "Permission flags",
                None,
                Some(joined.join(" ")),
                Some(joined.join(" ")),
                "runs-code",
                false,
                true,
            ));
        }
    }

    if parts.contains("cardNotes")
        && let Some(note) = file
            .get("cardNotes")
            .and_then(|v| v.as_str().or_else(|| v.get("note").and_then(|n| n.as_str())))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        let cur = existing
            .as_ref()
            .and_then(|a| a.get("note").and_then(|v| v.as_str()).map(ToString::to_string));
        changes.push(make_change(
            "cardNotes:note",
            "cardNotes",
            "Card note",
            cur,
            Some(note.to_string()),
            Some(note.to_string()),
            "none",
            true,
            false,
        ));
    }

    Json(json!({ "ok": true, "changes": changes }))
}

async fn agent_file_apply(
    State(state): State<AppState>,
    Json(req): Json<AgentFileApplyReq>,
) -> Json<Value> {
    if !is_safe_id(&req.agent_id) {
        return Json(json!({ "ok": false, "error": "invalid agentId" }));
    }
    let accepted: HashSet<String> = req.accept.into_iter().collect();
    let file = &req.file;
    let mut applied = Vec::new();
    let skipped: Vec<String> = Vec::new();
    let mut card = Map::new();
    let mut next_start = false;

    if let Some(c) = file.get("configs").and_then(|v| v.as_object()) {
        if accepted.contains("configs:name")
            && let Some(name) = file.get("name").cloned()
        {
            card.insert("name".to_string(), name);
            applied.push("configs:name".to_string());
        }
        if accepted.contains("configs:role")
            && let Some(role) = c.get("role").cloned()
        {
            card.insert("description".to_string(), role);
            applied.push("configs:role".to_string());
        }
        if accepted.contains("configs:goal")
            && let Some(goal) = c.get("goal").cloned()
        {
            card.insert("goal".to_string(), goal);
            applied.push("configs:goal".to_string());
        }
        if accepted.contains("configs:engine") {
            if let Some(p) = c.get("provider").cloned() {
                card.insert("provider".to_string(), p);
            }
            if let Some(m) = c.get("model").cloned() {
                card.insert("model".to_string(), m);
            }
            if let Some(f) = c.get("commandFlags").cloned() {
                card.insert("commandFlags".to_string(), f);
            }
            applied.push("configs:engine".to_string());
            next_start = true;
        }
        if accepted.contains("configs:tokenLimit")
            && let Some(tl) = c.get("tokenLimit").cloned()
        {
            let mut cfg_store = state.config.lock().unwrap();
            let cur = cfg_store.read();
            let mut limits = cur
                .extra
                .get("tokenLimits")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            limits.insert(req.agent_id.clone(), tl);
            let _ = cfg_store.update(json!({ "tokenLimits": limits }));
            applied.push("configs:tokenLimit".to_string());
        }
    }

    if accepted.contains("permissions:flags")
        && let Some(perms) = file.get("permissions").and_then(|v| v.as_object())
    {
        if let Some(flags) = perms.get("flags").cloned() {
            card.insert("permissionFlags".to_string(), flags);
        }
        if let Some(prov) = perms.get("provider").cloned() {
            card.insert("permissionsFor".to_string(), prov);
        }
        applied.push("permissions:flags".to_string());
        next_start = true;
    }

    if accepted.contains("cardNotes:note")
        && let Some(note) = file
            .get("cardNotes")
            .and_then(|v| v.as_str().or_else(|| v.get("note").and_then(|n| n.as_str())))
    {
        card.insert("note".to_string(), Value::String(note.to_string()));
        applied.push("cardNotes:note".to_string());
    }

    // Apply MCP servers into config.agentMcp[agentId]
    let mcp_ids: Vec<String> = accepted
        .iter()
        .filter(|id| id.starts_with("mcp:catalog:") || id.starts_with("mcp:custom:"))
        .cloned()
        .collect();
    if !mcp_ids.is_empty() {
        let mut cfg_store = state.config.lock().unwrap();
        let cur = cfg_store.read();
        let mut all_mcp = cur
            .extra
            .get("agentMcp")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut mine = all_mcp
            .get(&req.agent_id)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for id in &mcp_ids {
            if let Some(name) = id.strip_prefix("mcp:catalog:") {
                mine.insert(name.to_string(), Value::Bool(true));
                applied.push(id.clone());
            } else if let Some(rest) = id.strip_prefix("mcp:custom:") {
                let name = rest.split_once(':').map(|(_, n)| n).unwrap_or(rest);
                mine.insert(name.to_string(), Value::Bool(true));
                applied.push(id.clone());
            }
        }
        all_mcp.insert(req.agent_id.clone(), Value::Object(mine));
        let _ = cfg_store.update(json!({ "agentMcp": all_mcp }));
    }

    // Apply skills into config.agentSkills[agentId] & clear from agentSkillsOff
    let skill_ids: Vec<String> = accepted
        .iter()
        .filter(|id| id.starts_with("skill:catalog:") || id.starts_with("skill:custom:"))
        .cloned()
        .collect();
    if !skill_ids.is_empty() {
        let mut cfg_store = state.config.lock().unwrap();
        let cur = cfg_store.read();
        let mut all_skills = cur
            .extra
            .get("agentSkills")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut mine: Vec<String> = all_skills
            .get(&req.agent_id)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for id in &skill_ids {
            let name = id
                .strip_prefix("skill:catalog:")
                .or_else(|| id.strip_prefix("skill:custom:").and_then(|r| r.split_once(':').map(|(_, n)| n)))
                .unwrap_or(id.as_str())
                .to_string();
            if !mine.contains(&name) {
                mine.push(name);
            }
            applied.push(id.clone());
        }
        all_skills.insert(req.agent_id.clone(), json!(mine));
        let _ = cfg_store.update(json!({ "agentSkills": all_skills }));
    }

    // Apply memories into hive/agents/<id>/memory.md
    if accepted.contains("memories:content")
        && let Some(home) = state.harness_home()
        && let Some(content) = file
            .get("memories")
            .and_then(|v| v.as_str().or_else(|| v.get("content").and_then(|c| c.as_str())))
        && memory::write_agent_memory(&home, &req.agent_id, content).is_ok()
    {
        applied.push("memories:content".to_string());
    }

    // Apply automations into config.missions
    if let Some(autos) = file.get("automations").and_then(|v| v.as_array()) {
        let mut to_add = Vec::new();
        for (idx, item) in autos.iter().enumerate() {
            let id = format!("auto:{idx}");
            if accepted.contains(&id) {
                to_add.push((id, item.clone()));
            }
        }
        if !to_add.is_empty() {
            let mut cfg_store = state.config.lock().unwrap();
            let cur = cfg_store.read();
            let mut missions = cur
                .extra
                .get("missions")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for (aid, m) in to_add {
                missions.push(m);
                applied.push(aid);
            }
            let _ = cfg_store.update(json!({ "missions": missions }));
            state.emit("missions:updated", json!({ "ok": true }));
        }
    }

    if let Ok(val) = serde_json::to_value(state.config.lock().unwrap().read()) {
        state.emit("config:changed", val);
    }

    Json(json!({
        "ok": true,
        "applied": applied,
        "skipped": skipped,
        "nextStart": next_start,
        "card": Value::Object(card),
    }))
}

async fn agent_file_export(
    State(state): State<AppState>,
    Json(req): Json<AgentFileExportReq>,
) -> Json<Value> {
    let Some(agent) = find_agent_card(&state, &req.agent_id) else {
        return Json(json!({ "ok": false, "error": "agent not found" }));
    };
    let name = agent
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.agent_id)
        .to_string();
    let cfg = state.config.lock().unwrap().read();
    let parts: Vec<String> = if req.parts.is_empty() {
        vec!["configs".to_string(), "mcpServers".to_string(), "skills".to_string()]
    } else {
        req.parts.clone()
    };

    let mut out = Map::new();
    out.insert("spec".to_string(), json!("munder-difflin/agent@2"));
    out.insert("name".to_string(), json!(name));
    if let Some(desc) = req
        .description
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| agent.get("description").and_then(|v| v.as_str()))
    {
        out.insert("description".to_string(), json!(desc));
    }
    if let Some(author) = req.author.as_deref().filter(|s| !s.trim().is_empty()) {
        out.insert("author".to_string(), json!(author));
    }
    out.insert("parts".to_string(), json!(parts));

    if parts.iter().any(|p| p == "configs") {
        let mut c = Map::new();
        if let Some(r) = agent.get("description").or_else(|| agent.get("role")) {
            c.insert("role".to_string(), r.clone());
        }
        for k in ["goal", "character", "accent", "provider", "model", "capabilities", "isolate"] {
            if let Some(v) = agent.get(k) {
                c.insert(k.to_string(), v.clone());
            }
        }
        if let Some(tl) = cfg.extra.get("tokenLimits").and_then(|m| m.get(&req.agent_id)) {
            c.insert("tokenLimit".to_string(), tl.clone());
        }
        out.insert("configs".to_string(), Value::Object(c));
    }
    if parts.iter().any(|p| p == "mcpServers") {
        let cat: Vec<String> = cfg
            .extra
            .get("agentMcp")
            .and_then(|m| m.get(&req.agent_id))
            .and_then(|m| m.as_object())
            .map(|o| {
                o.iter()
                    .filter(|(_, v)| v.as_bool() == Some(true))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        out.insert("mcpServers".to_string(), json!({ "catalog": cat, "custom": [] }));
    }
    if parts.iter().any(|p| p == "skills") {
        let mut cat: Vec<String> = cfg
            .extra
            .get("agentSkills")
            .and_then(|m| m.get(&req.agent_id))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(extra) = req.extra_skill_paths {
            for s in extra {
                if !cat.contains(&s) {
                    cat.push(s);
                }
            }
        }
        out.insert("skills".to_string(), json!({ "catalog": cat, "custom": [] }));
    }
    if parts.iter().any(|p| p == "memories")
        && let Some(home) = state.harness_home()
        && let Ok(mem) = memory::read_agent_memory(&home, &req.agent_id)
        && !mem.trim().is_empty()
    {
        out.insert("memories".to_string(), json!({ "content": mem }));
    }
    if parts.iter().any(|p| p == "cardNotes")
        && let Some(note) = agent.get("note")
    {
        out.insert("cardNotes".to_string(), note.clone());
    }

    let file_val = Value::Object(out);
    let json_str = serde_json::to_string_pretty(&file_val).unwrap_or_else(|_| "{}".to_string());
    let file_name = format!("{}.agent.json", slugify(&name));
    let bytes = json_str.len();
    Json(json!({
        "ok": true,
        "file": file_val,
        "json": json_str,
        "fileName": file_name,
        "bytes": bytes,
    }))
}

async fn agent_file_skill_choices(
    Path(_agent_id): Path<String>,
    State(state): State<AppState>,
) -> Json<Value> {
    let mut out = Vec::new();
    let cfg = state.config.lock().unwrap().read();
    if let Some(custom) = cfg.extra.get("installedSkills").and_then(|v| v.as_array()) {
        for s in custom {
            if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
                out.push(json!({ "name": name, "path": s.get("path").cloned().unwrap_or_else(|| json!(name)) }));
            }
        }
    }
    Json(Value::Array(out))
}

pub fn user_templates_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("agent-templates")
}

async fn template_save(
    State(state): State<AppState>,
    Json(req): Json<TemplateSaveReq>,
) -> Json<Value> {
    let name = req.name.trim();
    if name.is_empty() {
        return Json(json!({ "ok": false, "error": "template name is required" }));
    }
    let dir = user_templates_dir(&state);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Json(json!({ "ok": false, "error": e.to_string() }));
    }
    let mut file = req.file;
    if let Some(obj) = file.as_object_mut() {
        obj.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(desc) = req.description.as_deref().filter(|s| !s.trim().is_empty()) {
            obj.insert("description".to_string(), Value::String(desc.to_string()));
        }
        let _ = req.accepted;
    }
    let id = slugify(name);
    let path = dir.join(format!("{id}.agent.json"));
    let Ok(text) = serde_json::to_string_pretty(&file) else {
        return Json(json!({ "ok": false, "error": "serialization failed" }));
    };
    if let Err(e) = std::fs::write(&path, text) {
        return Json(json!({ "ok": false, "error": e.to_string() }));
    }
    state.emit("agent:templates", json!({ "id": id }));
    Json(json!({ "ok": true, "id": id }))
}

async fn template_remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    if !is_safe_id(&id) {
        return Json(json!({ "ok": false, "error": "invalid template id" }));
    }
    let dir = user_templates_dir(&state);
    for cand in [
        dir.join(format!("{id}.agent.json")),
        dir.join(format!("{id}.json")),
    ] {
        if cand.is_file() {
            return match std::fs::remove_file(&cand) {
                Ok(()) => {
                    state.emit("agent:templates", json!({ "id": id, "removed": true }));
                    Json(json!({ "ok": true }))
                }
                Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
            };
        }
    }
    Json(json!({ "ok": false, "error": "template not found or built-in" }))
}

pub fn agent_file_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/agent-file/plan", post(agent_file_plan))
        .route("/api/v1/agent-file/apply", post(agent_file_apply))
        .route("/api/v1/agent-file/export", post(agent_file_export))
        .route(
            "/api/v1/agent-file/skill-choices/{id}",
            get(agent_file_skill_choices),
        )
        .route("/api/v1/templates/pro/save", post(template_save))
        .route("/api/v1/templates/pro/{id}", delete(template_remove))
}
