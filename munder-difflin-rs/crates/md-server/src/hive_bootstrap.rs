//! Hive filesystem bootstrap: writes `COMMANDS.md`, refreshes `fleet.json`,
//! initializes per-agent inbox/outbox/spawn-requests/memory directories, and
//! injects provider env vars, hooks, sandbox roots, and initial `seedPrompt`.

use crate::state::AppState;
use md_core::config::expand_tilde;
use md_core::repo_fs::is_safe_id;
use md_pty::manager::SpawnOptions;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn atomic_write_string(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn write_commands_md(hive_root: &Path) {
    let root_str = hive_root.to_string_lossy();
    let projects_hint = hive_root.join("projects").to_string_lossy().into_owned();
    let platform = md_core::host_node_platform();
    let arch = md_core::host_node_arch();

    let protocol_content = format!(
        r#"# Munder Difflin Agent Protocol

Host environment: `{platform}` (`{arch}`).
Hive root (`$HIVE_ROOT`): `{root_str}`.

1. **Inbox**: your inbox is `$HIVE_ROOT/agents/$AGENT_ID/inbox/`. Read every `.json` file there and move processed files into `inbox/.done/` so they are not re-delivered.
2. **Outbox**: to message another agent, the orchestrator (`god`), everyone (`broadcast`), or the human (`human`), write ONE `.json` file with a unique filename into `$HIVE_ROOT/agents/$AGENT_ID/outbox/<msg-id>.json`. Never write directly into another agent's folder.
   - Schema: `{{"to":"god|<agentId>|broadcast|human","act":"request|inform|done|ask|propose|note","subject":"...","body":"...","work":"<optional task key>"}}`
   - Valid `act` values: `request`, `inform`, `done`, `ask`, `propose`, `note`.
   - When asked to pause/wait or ask the user, use `"to": "human"` or set `humanQA` on a blocked card in `tasks.json`.
3. **Tasks (`$HIVE_ROOT/tasks.json`)**: track every unit of work cleanly. Set `"assignee"` to the owning agent id immediately on dispatch and update `"status"` (`todo`, `doing`, `done`, `blocked`). To request user input/action on a blocked card, push `{{"q":"...","askedAt":"<iso>","from":"$AGENT_ID"}}` to its `"humanQA"` array.
4. **Board (`$HIVE_ROOT/board.md`)**: shared plans and status. `god` is the primary scribe of `board.md`.
5. **Memory**: record durable facts and rules in `$HIVE_ROOT/agents/$AGENT_ID/memory.md` (`# Rules` section for invariants). Read siblings' `$HIVE_ROOT/agents/<id>/memory.md` (indexed in `$HIVE_ROOT/memory-index.md`) when work touches their area.
"#
    );

    let protocol_path = hive_root.join("PROTOCOL.md");
    if fs::read_to_string(&protocol_path).ok().as_deref() != Some(protocol_content.as_str()) {
        let _ = atomic_write_string(&protocol_path, &protocol_content);
    }

    let commands_content = format!(
        r#"# Munder Difflin Hive Protocol & Commands

Host environment: `{platform}` (`{arch}`).
Hive root (`$HIVE_ROOT`): `{root_str}`.
Every agent runs with environment variables: `AGENT_ID`, `AGENT_NAME`, `AGENT_DIR`, `HIVE_ROOT`, `MEMPALACE_PALACE_PATH`, `MD_SERVER_URL`, `MD_HOOK_URL`, `MD_HOOK_TOKEN`, `MD_BROKER_URL`, `MD_BROKER_TOKEN` (and user-defined custom secret env vars, plus `$AGENT_DIR/.broker`).

## 1. Receiving & Sending Mail (`inbox/` & `outbox/`)
- **Read mail**: `$HIVE_ROOT/agents/$AGENT_ID/inbox/*.json` -> move processed files into `inbox/.done/`.
- **Send mail**: write ONE `.json` file to `$HIVE_ROOT/agents/$AGENT_ID/outbox/<msg-id>.json`:
```json
{{
  "to": "god",
  "act": "inform",
  "subject": "Short subject",
  "body": "Message details",
  "needsHuman": false
}}
```
- The server drains `outbox/*.json` every 750ms, routes to the recipient's `inbox/`, and wakes their terminal.

## 2. Starting a Temp Worker (`spawn-requests/`)
- Orchestrator (`god`) writes ONE `.json` file into `$HIVE_ROOT/spawn-requests/<id>.json` (any lasting agent can also write to `$HIVE_ROOT/agents/$AGENT_ID/spawn-requests/<id>.json`):
```json
{{
  "name": "Research temp",
  "objective": "Clear, self-contained task description and where to write outputs",
  "cwd": "{projects_hint}/my-task",
  "provider": "opencode",
  "model": "",
  "isolate": true,
  "tokenCap": 0
}}
```
- Omit `"model"` (or leave empty) to use the default configured model. Omit `"provider"` to use the floor default.
- If a project folder does not exist yet, create one under `{projects_hint}/<short-name>` (`mkdir -p`) and pass that path in `"cwd"`.
- The harness spawns `worker-<id>`, briefs it with the objective, moves accepted requests to `.done/` and rejected ones to `.failed/` with `{{"error": "..."}}`.
- NEVER use CLI-internal subagent tools (`Task`, `Agent`, `General Task`) for delegated floor work — internal subagents are invisible on the office floor and bypass token caps, circuit breakers, and worker history. Always start managed temps via `spawn-requests/<id>.json`.
- Note: Respects Settings -> Autonomy & Budgets (`orchestratorMaySpawn` & `maxConcurrentWorkers`).

## 3. Fleet Status, Board, Tasks & Memory
- **Fleet status**: `$HIVE_ROOT/fleet.json` (live per-agent status, breaker state, tokens, inbox backlog) and `$HIVE_ROOT/registry.json`.
- **Shared board**: `$HIVE_ROOT/board.md` (`god` acts as scribe).
- **Kanban tasks**: `$HIVE_ROOT/tasks.json` (`{{"tasks":[{{"id":"...","title":"...","status":"todo|doing|done|blocked","assignee":"...","humanQA":[]}}]}}`).
- **Memory & Index**: `$HIVE_ROOT/agents/$AGENT_ID/memory.md` and `$HIVE_ROOT/memory-index.md`.
- **Integration Broker & Secrets**: use `$MD_BROKER_URL` with `Authorization: Bearer $MD_BROKER_TOKEN` (or read `$AGENT_DIR/.broker`):
  - `GET $MD_BROKER_URL/integrations` or `GET $MD_BROKER_URL/i` — list enabled integrations your token can reach.
  - `<METHOD> $MD_BROKER_URL/i/<id>/<path>` — call a configured REST API integration; the broker injects credentials upstream without revealing secrets.
  - `GET $MD_BROKER_URL/secrets` and `POST $MD_BROKER_URL/secrets/read` (`{{"name":"..."}}`) — list/read secrets if permitted; custom user secrets are also injected directly into your environment.
- Full protocol rules: `$HIVE_ROOT/PROTOCOL.md`.
"#,
    );

    let path = hive_root.join("COMMANDS.md");
    if fs::read_to_string(&path).ok().as_deref() != Some(commands_content.as_str()) {
        let _ = atomic_write_string(&path, &commands_content);
    }
}

fn copy_dir_missing_or_updated(src: &Path, dst: &Path) {
    let Ok(rd) = fs::read_dir(src) else {
        return;
    };
    if fs::create_dir_all(dst).is_err() {
        return;
    }
    for entry in rd.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy() == ".DS_Store" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_dir_missing_or_updated(&from, &to);
        } else if from.is_file()
            && let Ok(bytes) = fs::read(&from)
            && fs::read(&to).ok().as_deref() != Some(bytes.as_slice())
        {
            let _ = fs::write(&to, &bytes);
        }
    }
}

fn ensure_agent_skills(agent_dir: &Path) {
    let Some(res_root) = crate::routes_catalog::resources_root() else {
        return;
    };
    let src_skills = res_root.join("skills");
    if src_skills.is_dir() {
        copy_dir_missing_or_updated(&src_skills, &agent_dir.join(".claude").join("skills"));
    }
}

fn write_executable_script(path: &Path, content: &str) {
    if fs::read_to_string(path).ok().as_deref() != Some(content)
        && atomic_write_string(path, content).is_ok()
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
    }
}

fn ensure_hive_bin_helpers(hive_root: &Path) {
    let bin_dir = hive_root.join("bin");
    if fs::create_dir_all(&bin_dir).is_err() {
        return;
    }
    let md_limits = r#"#!/bin/sh
set -eu
URL="${MD_SERVER_URL:-http://127.0.0.1:3110}/api/v1/usage/limits"
if [ "${1:-}" = "--json" ]; then
  exec curl -sf "$URL"
fi
curl -sf "$URL" || echo '{"ok":false,"error":"usage limits unavailable"}'
"#;
    let md_models = r#"#!/bin/sh
set -eu
CFG=$(curl -sf "${MD_SERVER_URL:-http://127.0.0.1:3110}/api/v1/config" || echo '{}')
if [ "${1:-}" = "--json" ]; then
  printf '%s\n' "$CFG"
  exit 0
fi
printf 'Configured engines/models (from %s):\n%s\n' "${MD_SERVER_URL:-http://127.0.0.1:3110}" "$CFG"
"#;
    write_executable_script(&bin_dir.join("md-limits"), md_limits);
    write_executable_script(&bin_dir.join("md-models"), md_models);
}

pub fn ensure_agent_hive_dirs(hive_root: &Path, agent_id: &str, name: &str) {
    let _ = fs::create_dir_all(hive_root.join("spawn-requests"));
    let _ = fs::create_dir_all(hive_root.join("hire-requests"));
    let _ = fs::create_dir_all(hive_root.join("compact-requests"));
    let _ = fs::create_dir_all(hive_root.join("clear-requests"));
    ensure_hive_bin_helpers(hive_root);
    if !is_safe_id(agent_id) {
        return;
    }
    let agent_dir = hive_root.join("agents").join(agent_id);
    for sub in [
        "inbox",
        "inbox/.done",
        "outbox",
        "outbox/.sent",
        "spawn-requests",
    ] {
        let _ = fs::create_dir_all(agent_dir.join(sub));
    }
    let mem = agent_dir.join("memory.md");
    if !mem.exists() {
        let _ = atomic_write_string(&mem, &format!("# Memory: {name}\n"));
    }
    let handoff = agent_dir.join("handoff.md");
    if !handoff.exists() {
        let _ = atomic_write_string(
            &handoff,
            &format!("# Handoff: {name}\n\n_One section per piece of work. The app keeps the newest 12._\n"),
        );
    }
}

pub fn sync_fleet_json(state: &AppState) {
    let Some(home) = state.harness_home() else {
        return;
    };
    state.sync_hive_home();
    let hive_root = home.join("hive");
    if fs::create_dir_all(&hive_root).is_err() {
        return;
    }
    write_commands_md(&hive_root);

    let reg = state.hive.lock().unwrap().registry();
    let contexts = state.hooks.context_by_id.lock().unwrap().clone();
    let models = state.hooks.model_by_id.lock().unwrap().clone();
    let sessions = state.hooks.session_by_id.lock().unwrap().clone();
    let breakers = state.breaker.lock().unwrap();
    let ptys = state.pty.list();
    let usage_by_id: std::collections::HashMap<String, md_hive::usage::AgentUsage> =
        crate::routes_usage_memory_kg::collect_agent_usage_rows(state)
            .into_iter()
            .collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut god_id: Option<String> = None;
    let mut rows = Vec::new();
    if let Some(agents) = reg.get("agents").and_then(|v| v.as_object()) {
        let mut keys: Vec<&String> = agents.keys().collect();
        keys.sort();
        for id in keys {
            let info = &agents[id];
            let is_god = info.get("isGod").and_then(|v| v.as_bool()).unwrap_or(false) || id == "god";
            if is_god && god_id.is_none() {
                god_id = Some(id.clone());
            }
            let is_temp = info.get("isTemp").and_then(|v| v.as_bool()).unwrap_or(false)
                || id.starts_with("worker-");
            let archived = info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false);
            let sleeping = info.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false);
            let name = info.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            if !archived {
                ensure_agent_hive_dirs(&hive_root, id, name);
            }
            let pty_running = ptys.iter().any(|p| p.id == format!("pty-{id}"));
            let stopped = !pty_running && !sleeping;
            if archived || stopped {
                continue;
            }
            let provider = info.get("provider").and_then(|v| v.as_str()).unwrap_or("claude");
            let cwd_str = info.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
            let hook_sid = sessions.get(id).map(String::as_str);
            let started_at = info.get("startedAt").and_then(|v| v.as_u64());
            let oc_usage = if provider == "opencode" && (!cwd_str.is_empty() || hook_sid.is_some())
            {
                let expanded = if cwd_str.is_empty() {
                    String::new()
                } else {
                    expand_tilde(cwd_str).to_string_lossy().into_owned()
                };
                crate::routes_usage_memory_kg::read_opencode_usage_for_since(
                    &expanded,
                    hook_sid,
                    !sleeping,
                    started_at,
                )
            } else {
                None
            };
            let u = usage_by_id
                .get(id)
                .cloned()
                .or_else(|| oc_usage.as_ref().map(|(x, _)| x.clone()));
            let tokens = u
                .as_ref()
                .map(|x| x.input_tokens + x.output_tokens + x.cache_read_tokens + x.cache_write_tokens)
                .unwrap_or(0);
            let usd = u
                .as_ref()
                .map(|x| (x.estimated_cost_usd * 10_000.0).round() / 10_000.0)
                .unwrap_or(0.0);
            let ctx = contexts.get(id).cloned().or_else(|| {
                oc_usage.as_ref().and_then(|(oc_u, latest)| {
                    latest.map(|tok| {
                        let model_name = oc_u
                            .model
                            .as_deref()
                            .or_else(|| models.get(id).map(String::as_str))
                            .or_else(|| info.get("model").and_then(|v| v.as_str()))
                            .unwrap_or("");
                        let limit = crate::routes_usage_memory_kg::opencode_model_context_limit(
                            None,
                            model_name,
                        );
                        md_hive::hooks::ContextSnapshot {
                            agent_id: id.clone(),
                            tokens: tok,
                            limit,
                            ts: now_ms,
                        }
                    })
                })
            });
            let ctx_pct = ctx.as_ref().and_then(|c| {
                if c.limit > 0 {
                    Some((((c.tokens as f64 / c.limit as f64) * 100.0).round() as u64).min(100))
                } else {
                    None
                }
            });
            let inbox_backlog = state.hive.lock().unwrap().inbox(id).len();
            let status = if sleeping {
                "sleeping"
            } else {
                info.get("status").and_then(|v| v.as_str()).unwrap_or("idle")
            };
            rows.push(json!({
                "id": id,
                "name": name,
                "role": info.get("role").and_then(|v| v.as_str()).unwrap_or(""),
                "provider": provider,
                "model": models
                    .get(id)
                    .cloned()
                    .or_else(|| u.as_ref().and_then(|x| x.model.clone()))
                    .or_else(|| info.get("model").and_then(|v| v.as_str()).map(ToString::to_string)),
                "status": status,
                "sleeping": sleeping,
                "stopped": stopped,
                "archived": archived,
                "onHold": info.get("onHold").and_then(|v| v.as_bool()).unwrap_or(false),
                "isGod": is_god,
                "isTemp": is_temp,
                "cwd": info.get("cwd").cloned().unwrap_or(Value::Null),
                "breaker": breakers.state_for(id).level,
                "tokens": tokens,
                "usd": usd,
                "sessionUsd": usd,
                "contextTokens": ctx.as_ref().map(|c| c.tokens),
                "contextLimit": ctx.as_ref().map(|c| c.limit),
                "contextPct": ctx_pct,
                "inboxBacklog": inbox_backlog,
            }));
        }
    }

    let doc = json!({
        "ts": now_ms,
        "godId": god_id,
        "hiveRoot": hive_root.to_string_lossy().into_owned(),
        "agents": rows,
    });
    let serialized = serde_json::to_string_pretty(&doc).unwrap_or_default();
    let path = hive_root.join("fleet.json");
    if fs::read_to_string(&path).ok().as_deref() != Some(serialized.as_str()) {
        let _ = atomic_write_string(&path, &serialized);
    }
}

fn install_claude_hooks_settings(agent_dir: &Path) -> Option<PathBuf> {
    let settings_path = agent_dir.join("claude-settings.json");
    let hook_cmd = "curl -sS -m 5 -X POST \"$MD_HOOK_URL\" -H \"Authorization: Bearer $MD_HOOK_TOKEN\" -H \"Content-Type: application/json\" --data-binary @- || true";
    let hook_entry = |matcher: Option<&str>| -> Value {
        let mut obj = Map::new();
        if let Some(m) = matcher {
            obj.insert("matcher".to_string(), json!(m));
        }
        obj.insert(
            "hooks".to_string(),
            json!([{ "type": "command", "command": hook_cmd }]),
        );
        Value::Object(obj)
    };
    let doc = json!({
        "hooks": {
            "SessionStart": [hook_entry(None)],
            "UserPromptSubmit": [hook_entry(None)],
            "PreToolUse": [hook_entry(Some("*"))],
            "PostToolUse": [hook_entry(Some("*"))],
            "PreCompact": [hook_entry(None)],
            "PostCompact": [hook_entry(None)],
            "Stop": [hook_entry(None)],
            "Notification": [hook_entry(None)],
        }
    });
    if let Ok(text) = serde_json::to_string_pretty(&doc)
        && atomic_write_string(&settings_path, &text).is_ok()
    {
        return Some(settings_path);
    }
    None
}

fn install_opencode_config_content(
    agent_dir: &Path,
    hive_root: &Path,
    auto_mode: bool,
) -> Option<String> {
    if fs::create_dir_all(agent_dir).is_err() {
        return None;
    }
    let plugin_dir = agent_dir.join("hive-plugin");
    let pkg_json_path = plugin_dir.join("package.json");
    let plugin_path = plugin_dir.join("index.mjs");
    let pkg_src = r#"{
  "name": "munder-difflin-hive",
  "version": "1.0.0",
  "type": "module",
  "main": "./index.mjs",
  "exports": {
    ".": "./index.mjs"
  }
}
"#;
    let plugin_src = r#"import fs from "node:fs";
import os from "node:os";
import path from "node:path";

let cachedModelsMtimeMs = -1;
let cachedModelLimits = new Map();

function loadModelLimits() {
  try {
    const cacheBase = process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
    const modelsPath = path.join(cacheBase, "opencode", "models.json");
    const st = fs.statSync(modelsPath);
    if (st.mtimeMs === cachedModelsMtimeMs) return cachedModelLimits;
    const raw = JSON.parse(fs.readFileSync(modelsPath, "utf8"));
    const next = new Map();
    if (raw && typeof raw === "object") {
      for (const [provId, provVal] of Object.entries(raw)) {
        const models = provVal?.models;
        if (!models || typeof models !== "object") continue;
        for (const [modelId, modelVal] of Object.entries(models)) {
          const ctx = modelVal?.limit?.context;
          if (typeof ctx === "number" && ctx > 0) {
            next.set(`${provId}/${modelId}`, ctx);
            if (provId === "opencode" || provId === "anthropic" || !next.has(modelId)) {
              next.set(modelId, ctx);
            }
          }
        }
      }
    }
    cachedModelsMtimeMs = st.mtimeMs;
    cachedModelLimits = next;
  } catch {}
  return cachedModelLimits;
}

function contextLimitOf(providerId, modelId) {
  const limits = loadModelLimits();
  const mid = typeof modelId === "string" ? modelId.trim() : "";
  const pid = typeof providerId === "string" ? providerId.trim() : "";
  if (pid && mid && limits.has(`${pid}/${mid}`)) return limits.get(`${pid}/${mid}`);
  if (mid && limits.has(mid)) return limits.get(mid);
  const slashIdx = mid.indexOf("/");
  if (slashIdx > 0) {
    const short = mid.slice(slashIdx + 1);
    if (limits.has(short)) return limits.get(short);
  }
  return /1m/i.test(mid) ? 1000000 : 200000;
}

async function postHook(payload) {
  try {
    const url = process.env.MD_HOOK_URL;
    const tok = process.env.MD_HOOK_TOKEN;
    if (!url || !tok) return;
    payload.agent_id = payload.agent_id || process.env.AGENT_ID || undefined;
    await fetch(url, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "authorization": `Bearer ${tok}`,
      },
      body: JSON.stringify(payload),
    }).catch(() => {});
  } catch {}
}

export const HiveBridge = async (ctx) => {
  let stopTimer = null;
  let lastStopAt = 0;
  let lastCompactAt = 0;
  let activeSessionId = undefined;
  const modelBySession = new Map();
  const compactionMsgs = new Set();

  const clearStopTimer = () => {
    if (stopTimer) {
      clearTimeout(stopTimer);
      stopTimer = null;
    }
  };
  const sidOf = (obj) =>
    obj?.sessionID ||
    obj?.data?.sessionID ||
    obj?.info?.sessionID ||
    obj?.properties?.sessionID ||
    obj?.properties?.info?.sessionID ||
    obj?.message?.sessionID ||
    activeSessionId;

  const emitStop = (sessionId) => {
    clearStopTimer();
    const now = Date.now();
    if (now - lastStopAt < 1500) return Promise.resolve();
    lastStopAt = now;
    return postHook({ hook_event_name: "Stop", session_id: sessionId });
  };
  const emitPostCompact = (sessionId) => {
    clearStopTimer();
    const now = Date.now();
    if (now - lastCompactAt < 1500) return Promise.resolve();
    lastCompactAt = now;
    return postHook({ hook_event_name: "PostCompact", session_id: sessionId });
  };

  void postHook({ hook_event_name: "SessionStart" });

  const handleEvent = async (ev) => {
    try {
      const t = ev?.type;
      if (!t || t.endsWith(".delta")) return;
      const d = ev?.data ?? ev?.properties ?? {};
      const sid = sidOf(ev);
      if (sid) activeSessionId = sid;

      if (t === "session.inbox.enqueued") {
        const item = d?.item;
        if (item?.type === "user") {
          clearStopTimer();
          lastStopAt = 0;
          const text = item?.payload?.text;
          if (typeof text === "string" && text.trim()) {
            await postHook({
              hook_event_name: "UserPromptSubmit",
              session_id: sid,
              prompt: text.slice(-4096),
            });
          }
        }
      } else if (t === "session.step.started") {
        clearStopTimer();
        if (d?.model?.id || d?.modelID) {
          modelBySession.set(sid || "", {
            providerID: d?.model?.providerID || d?.providerID,
            modelID: d?.model?.id || d?.modelID,
          });
        }
        if (d?.agent === "compaction" || d?.mode === "compaction") {
          if (d?.assistantMessageID) compactionMsgs.add(d.assistantMessageID);
          lastCompactAt = 0;
          await postHook({ hook_event_name: "PreCompact", session_id: sid });
        }
      } else if (t === "session.compacting" || t === "session.compaction.started") {
        clearStopTimer();
        lastCompactAt = 0;
        await postHook({ hook_event_name: "PreCompact", session_id: sid });
      } else if (
        t === "session.compacted" ||
        t === "session.compaction.completed" ||
        t === "session.compaction.ended"
      ) {
        await emitPostCompact(sid);
      } else if (t === "session.step.ended") {
        clearStopTimer();
        if (d?.assistantMessageID && compactionMsgs.delete(d.assistantMessageID)) {
          await emitPostCompact(sid);
        }
        const tok = d?.tokens;
        if (tok && typeof tok === "object") {
          const sum =
            (tok.input || 0) +
            (tok.output || 0) +
            (tok.reasoning || 0) +
            (tok.cache?.read || 0) +
            (tok.cache?.write || 0);
          const total = typeof tok.total === "number" && tok.total > 0 ? tok.total : sum;
          if (total > 0) {
            const tracked = modelBySession.get(sid || "");
            const modelId = d?.model?.id || d?.modelID || tracked?.modelID;
            const provId = d?.model?.providerID || d?.providerID || tracked?.providerID;
            const limit = contextLimitOf(provId, modelId);
            await postHook({
              hook_event_name: "Status",
              session_id: sid,
              model: modelId || undefined,
              context_window: {
                total_input_tokens: total,
                context_window_size: limit,
              },
            });
          }
        }
      } else if (
        t === "session.execution.succeeded" ||
        t === "session.execution.failed" ||
        t === "session.execution.aborted" ||
        t === "session.idle" ||
        (t === "session.status" && d?.status?.type === "idle")
      ) {
        await emitStop(sid);
      } else if (t === "session.status" && d?.status?.type !== "idle") {
        clearStopTimer();
      } else if (t === "message.updated") {
        const info = d?.info;
        if (info?.mode === "compaction" || info?.agent === "compaction") {
          clearStopTimer();
          if (info?.time?.completed) {
            await emitPostCompact(sid);
          } else {
            lastCompactAt = 0;
            await postHook({ hook_event_name: "PreCompact", session_id: sid });
          }
        }
        if (info?.role === "assistant" && info?.tokens && (info?.time?.completed || info?.finish)) {
          const tok = info.tokens;
          const sum =
            (tok.input || 0) +
            (tok.output || 0) +
            (tok.reasoning || 0) +
            (tok.cache?.read || 0) +
            (tok.cache?.write || 0);
          const total = typeof tok.total === "number" && tok.total > 0 ? tok.total : sum;
          if (total > 0) {
            const limit = contextLimitOf(
              info?.model?.providerID || info?.providerID,
              info?.model?.id || info?.modelID,
            );
            await postHook({
              hook_event_name: "Status",
              session_id: sid,
              model: info?.model?.id || info?.modelID || undefined,
              context_window: {
                total_input_tokens: total,
                context_window_size: limit,
              },
            });
          }
        }
        if (info?.role === "assistant" && info?.time?.completed && info?.finish && info.finish !== "tool-calls") {
          clearStopTimer();
          stopTimer = setTimeout(() => {
            stopTimer = null;
            void emitStop(sid);
          }, 200);
        }
      }
    } catch {}
  };

  const sub = ctx?.event?.subscribe?.();
  if (sub && typeof sub[Symbol.asyncIterator] === "function") {
    void (async () => {
      try {
        for await (const ev of sub) {
          await handleEvent(ev);
        }
      } catch {}
    })();
  }

  try {
    ctx?.tool?.hook?.("execute.before", async (input) => {
      clearStopTimer();
      lastStopAt = 0;
      try {
        await postHook({
          hook_event_name: "PreToolUse",
          session_id: sidOf(input),
          tool_name: input?.tool || input?.info?.name || input?.name || "tool",
          tool_input: input?.args || input?.input || {},
        });
      } catch {}
    });
    ctx?.tool?.hook?.("execute.after", async (input) => {
      clearStopTimer();
      try {
        await postHook({
          hook_event_name: "PostToolUse",
          session_id: sidOf(input),
          tool_name: input?.tool || input?.info?.name || input?.name || "tool",
          tool_input: input?.args || input?.input || {},
        });
      } catch {}
    });
  } catch {}

  return {
    event: async (input) => handleEvent(input?.event ?? input),
  };
};

export default {
  id: "munder-difflin-hive",
  setup: HiveBridge,
};
"#;
    let _ = atomic_write_string(&pkg_json_path, pkg_src);
    let _ = atomic_write_string(&plugin_path, plugin_src);
    let oc_dir = agent_dir.join(".opencode");
    if fs::create_dir_all(&oc_dir).is_ok() {
        let tui_path = oc_dir.join("tui.json");
        if !tui_path.exists() {
            let _ = atomic_write_string(
                &tui_path,
                "{\n  \"$schema\": \"https://opencode.ai/tui.json\",\n  \"theme\": \"system\"\n}\n",
            );
        }
    }

    let mut cfg = Map::new();
    cfg.insert("autoupdate".to_string(), Value::Bool(false));
    let mut plugin_url = String::from("file://");
    plugin_url.push_str(&plugin_dir.to_string_lossy());
    cfg.insert("plugin".to_string(), json!([plugin_url]));
    cfg.insert(
        "instructions".to_string(),
        json!([
            agent_dir.join("identity.md").to_string_lossy().into_owned(),
            hive_root.join("PROTOCOL.md").to_string_lossy().into_owned(),
            hive_root.join("COMMANDS.md").to_string_lossy().into_owned(),
        ]),
    );
    if auto_mode {
        cfg.insert(
            "permission".to_string(),
            json!({
                "*": "allow",
                "edit": "allow",
                "bash": "allow",
                "webfetch": "allow",
                "external_directory": "allow",
                "doom_loop": "allow",
                "question": "deny"
            }),
        );
    }
    serde_json::to_string(&Value::Object(cfg)).ok()
}

fn install_codex_home(home: &Path, hive_root: &Path, agent_dir: &Path) -> Option<PathBuf> {
    let codex_dir = agent_dir.join("codex-home");
    if fs::create_dir_all(&codex_dir).is_err() {
        return None;
    }
    if let Some(user_home) = dirs::home_dir() {
        let global_codex = user_home.join(".codex");
        let auth_src = global_codex.join("auth.json");
        let auth_dst = codex_dir.join("auth.json");
        if auth_src.is_file() && !auth_dst.exists() {
            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink(&auth_src, &auth_dst);
            }
            #[cfg(not(unix))]
            {
                let _ = fs::copy(&auth_src, &auth_dst);
            }
        }
        let cfg_src = global_codex.join("config.toml");
        let mut cfg_text = fs::read_to_string(&cfg_src).unwrap_or_default();
        if !cfg_text.contains("[sandbox_workspace_write]") {
            let h_str = home.to_string_lossy().replace('"', "\\\"");
            let hr_str = hive_root.to_string_lossy().replace('"', "\\\"");
            cfg_text.push_str(&format!(
                "\n[sandbox_workspace_write]\nwritable_roots = [\"{h_str}\", \"{hr_str}\"]\n"
            ));
        }
        let _ = atomic_write_string(&codex_dir.join("config.toml"), &cfg_text);
    }
    let hook_cmd = "curl -sS -m 5 -X POST \"$MD_HOOK_URL\" -H \"Authorization: Bearer $MD_HOOK_TOKEN\" -H \"Content-Type: application/json\" --data-binary @- || true";
    let hooks_doc = json!({
        "hooks": {
            "SessionStart": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "PreToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "PostToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "PreCompact": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "PostCompact": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
        }
    });
    if let Ok(text) = serde_json::to_string_pretty(&hooks_doc) {
        let _ = atomic_write_string(&codex_dir.join("hooks.json"), &text);
    }
    Some(codex_dir)
}

fn install_gemini_hooks(agent_dir: &Path) -> Option<PathBuf> {
    let gemini_dir = agent_dir.join(".gemini-hive");
    let settings_path = gemini_dir.join("system-settings.json");
    let hook_cmd = "curl -sS -m 5 -X POST \"$MD_HOOK_URL\" -H \"Authorization: Bearer $MD_HOOK_TOKEN\" -H \"Content-Type: application/json\" --data-binary @- || true";
    let hook_entry = |name: &str, matcher: Option<&str>| -> Value {
        let mut obj = Map::new();
        if let Some(m) = matcher {
            obj.insert("matcher".to_string(), json!(m));
        }
        obj.insert("sequential".to_string(), json!(true));
        obj.insert(
            "hooks".to_string(),
            json!([{
                "name": format!("munder-hive-{name}"),
                "type": "command",
                "command": hook_cmd,
                "timeout": 30000
            }]),
        );
        Value::Object(obj)
    };
    let doc = json!({
        "hooksConfig": { "enabled": true, "notifications": false },
        "hooks": {
            "SessionStart": [hook_entry("session-start", None)],
            "BeforeAgent": [hook_entry("before-agent", None)],
            "BeforeTool": [hook_entry("before-tool", Some(".*"))],
            "AfterTool": [hook_entry("after-tool", Some(".*"))],
            "AfterAgent": [hook_entry("after-agent", None)],
        }
    });
    if let Ok(text) = serde_json::to_string_pretty(&doc)
        && atomic_write_string(&settings_path, &text).is_ok()
    {
        return Some(settings_path);
    }
    None
}

fn install_pi_hooks(agent_dir: &Path) -> Option<PathBuf> {
    let pi_dir = agent_dir.join(".pi-agent");
    let ext_path = pi_dir.join("extensions").join("hive-bridge.js");
    let ext_src = r#"async function postHook(payload) {
  try {
    const url = process.env.MD_HOOK_URL;
    const tok = process.env.MD_HOOK_TOKEN;
    if (!url || !tok) return;
    payload.agent_id = payload.agent_id || process.env.AGENT_ID || undefined;
    await fetch(url, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "authorization": `Bearer ${tok}`,
      },
      body: JSON.stringify(payload),
    }).catch(() => {});
  } catch {}
}

export default function (pi) {
  void postHook({ hook_event_name: "SessionStart" });
  pi.on("tool_call", async (event) => {
    await postHook({ hook_event_name: "PreToolUse", tool_name: event?.toolName || event?.tool });
  });
  pi.on("tool_result", async (event) => {
    await postHook({ hook_event_name: "PostToolUse", tool_name: event?.toolName || event?.tool });
  });
  pi.on("agent_end", async () => {
    await postHook({ hook_event_name: "Stop" });
  });
}
"#;
    if atomic_write_string(&ext_path, ext_src).is_err() {
        return None;
    }
    let manifest = json!({
        "name": "munder-hive-bridge",
        "version": "0.3.2",
        "main": "extensions/hive-bridge.js",
        "auto": true
    });
    if let Ok(text) = serde_json::to_string_pretty(&manifest) {
        let _ = atomic_write_string(&pi_dir.join("extensions.json"), &text);
    }
    Some(pi_dir)
}

pub fn build_seed_prompt(
    hive_root: &Path,
    agent_id: &str,
    name: &str,
    is_god: bool,
    role: Option<&str>,
    cwd: &str,
) -> String {
    let hr = hive_root.to_string_lossy();
    let agent_dir = format!("{hr}/agents/{agent_id}");
    let projects_dir = format!("{hr}/projects");
    let platform = md_core::host_node_platform();
    let arch = md_core::host_node_arch();
    let is_temp = role == Some("temp") || agent_id.starts_with("worker-");
    let shared_memory = format!(
        "Shared memory: every agent on this floor keeps durable notes at `{hr}/agents/<id>/memory.md`, and `{hr}/memory-index.md` lists them with path, size, and last update. Read any sibling's notes when a task touches their work; write only your own `{agent_dir}/memory.md` (with `# Rules` for hard constraints)."
    );

    if is_god {
        format!(
            r#"YOU ARE THE ORCHESTRATOR (AGENT_ID: `god`, name: {name}). Host environment: `{platform}` (`{arch}`). Workspace: `{cwd}`. HIVE_ROOT: `{hr}`.
You run the office; you coordinate, delegate, track state, and steward quality rather than doing raw implementation yourself unless no worker fits.
1. KNOW THE FLOOR. Maintain an accurate view of all active/idle/sleeping/archived agents (`{hr}/registry.json` for who exists, `{hr}/fleet.json` for live status, breaker level, tokens, context, and inbox backlog). Drain `{agent_dir}/inbox/` continually (move handled `.json` files to `inbox/.done/`).
2. DELEGATE. Prefer capable agents already on the floor (`{hr}/agents/<id>/inbox/`) or spawn a temp worker (`{hr}/spawn-requests/<slug>.json`) with a clear four-part contract: OBJECTIVE, OUTPUT, TOOLS, BOUNDARIES.
3. OWN BOARD & TASKS. You are the primary scribe of `{hr}/board.md`. Keep `{hr}/tasks.json` accurate: set `"assignee"` immediately on dispatch, update `"status"` (`todo|doing|done|blocked`), and push `{{"q":"...","askedAt":"<iso>","from":"<agentId>"}}` into `"humanQA"` when human input is needed.
4. PROJECT FOLDERS: If work has no folder yet, create one under `{projects_dir}/<short-name>` and pass it as `"cwd"`.
5. STARTING A TEMP: Write ONE JSON file into `{hr}/spawn-requests/<id>.json`:
   `{{"name":"...","objective":"...","cwd":"{cwd}","provider":"opencode","model":"","isolate":true,"tokenCap":0}}`
   (omit `"model"` to use defaults; accepted requests move to `spawn-requests/.done/`, rejections to `.failed/`). Respects Settings -> Autonomy & Budgets (`orchestratorMaySpawn` & `maxConcurrentWorkers`).
6. MESSAGING & ESCALATION: Send messages by writing ONE `.json` file to `{agent_dir}/outbox/<msg-id>.json` (`{{"to":"<agentId|broadcast|human>","act":"request|inform|done|ask|propose|note","subject":"...","body":"..."}}`). Read `{hr}/PROTOCOL.md` and `{hr}/COMMANDS.md` for full schemas.
7. {shared_memory}"#
        )
    } else if is_temp {
        let job_line = role
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|r| format!("Your objective: {r}\n"))
            .unwrap_or_default();
        format!(
            "[HIVE BOOTSTRAP]\nYour AGENT_ID is `temp` (`{agent_id}`, name: {name}). Host: `{platform}` (`{arch}`). Workspace: `{cwd}`. HIVE_ROOT: `{hr}`.\n{job_line}YOU ARE A TEMP: complete this one job cleanly, then send ONE outbox JSON file to `{agent_dir}/outbox/done-{agent_id}.json` with `{{\"to\":\"god\",\"act\":\"done\",\"subject\":\"Result\",\"body\":\"<concise summary and files written>\"}}`.\n- Full protocol: `{hr}/PROTOCOL.md` & `{hr}/COMMANDS.md`.\n- {shared_memory}"
        )
    } else {
        let role_line = role
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|r| format!("Your role/goal: {r}\n"))
            .unwrap_or_default();
        format!(
            "[HIVE BOOTSTRAP]\nYour AGENT_ID is `{agent_id}` (name: {name}). Host: `{platform}` (`{arch}`). Workspace: `{cwd}`. HIVE_ROOT: `{hr}`.\n{role_line}- Read `{hr}/PROTOCOL.md`, `{hr}/COMMANDS.md`, and `{agent_dir}/memory.md`.\n- Check mail in `{agent_dir}/inbox/` (move processed `.json` files to `inbox/.done/`).\n- Send replies/reports by writing a `.json` file to `{agent_dir}/outbox/<id>.json` with `{{\"to\":\"god\",\"act\":\"done|inform|ask\",\"subject\":\"...\",\"body\":\"...\"}}`.\n- For side tasks you can write `{agent_dir}/spawn-requests/<id>.json` (`{{\"objective\":\"...\",\"cwd\":\"{cwd}\"}}`).\n- Shared board/tasks: `{hr}/board.md`, `{hr}/tasks.json`.\n- {shared_memory}"
        )
    }
}

pub fn prepare_agent_spawn(
    state: &AppState,
    opts: &mut SpawnOptions,
    agent_id: &str,
    name: &str,
    provider: &str,
    is_god: bool,
    role: Option<&str>,
) -> Option<String> {
    let home = state.harness_home()?;
    state.sync_hive_home();
    let hive_root = home.join("hive");
    let _ = fs::create_dir_all(hive_root.join("projects"));
    let _ = state.hive.lock().unwrap().ensure_hive();
    write_commands_md(&hive_root);
    ensure_agent_hive_dirs(&hive_root, agent_id, name);

    let agent_dir_path = hive_root.join("agents").join(agent_id);
    if is_safe_id(agent_id) {
        let role_label = if is_god {
            "orchestrator (god)"
        } else {
            role.filter(|s| !s.trim().is_empty()).unwrap_or("agent")
        };
        let cwd_disp = expand_tilde(&opts.cwd).to_string_lossy().into_owned();
        let identity_body = if is_god {
            format!(
                "# {name} ({agent_id})\n- Role: {role_label}\n- Working directory: {cwd_disp}\n- You are the **god / orchestrator**. Coordinate the floor, delegate execution to agents or temps, and own decomposition, sign-offs, and final QA.\n- Monitor the team with `fleet.json` and `registry.json`; full command reference in `COMMANDS.md`.\n"
            )
        } else {
            format!(
                "# {name} ({agent_id})\n- Role: {role_label}\n- Working directory: {cwd_disp}\n"
            )
        };
        let id_path = agent_dir_path.join("identity.md");
        if fs::read_to_string(&id_path).ok().as_deref() != Some(identity_body.as_str()) {
            let _ = atomic_write_string(&id_path, &identity_body);
        }
        ensure_agent_skills(&agent_dir_path);
    }

    // Ensure agent exists in registry.json
    if is_safe_id(agent_id) {
        let mut entry = Map::new();
        entry.insert("id".to_string(), Value::String(agent_id.to_string()));
        entry.insert("name".to_string(), Value::String(name.to_string()));
        entry.insert("provider".to_string(), Value::String(provider.to_string()));
        entry.insert(
            "cwd".to_string(),
            Value::String(expand_tilde(&opts.cwd).to_string_lossy().into_owned()),
        );
        entry.insert("isGod".to_string(), Value::Bool(is_god));
        entry.insert("archived".to_string(), Value::Bool(false));
        entry.insert("stopped".to_string(), Value::Bool(false));
        if opts.resume != Some(true) {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            entry.insert("startedAt".to_string(), Value::Number(now_ms.into()));
        }
        let effective_role = role
            .filter(|s| !s.trim().is_empty())
            .or(if is_god {
                Some("orchestrator (god)")
            } else {
                None
            });
        if let Some(r) = effective_role {
            entry.insert("role".to_string(), Value::String(r.to_string()));
        }
        let _ = state.hive.lock().unwrap().register_worker(agent_id, entry);
    }

    let _ = md_hive::memory::write_memory_index(&home);

    let port = state.port;
    opts.env
        .entry("AGENT_ID".to_string())
        .or_insert_with(|| agent_id.to_string());
    opts.env
        .entry("AGENT_NAME".to_string())
        .or_insert_with(|| name.to_string());
    opts.env
        .entry("AGENT_DIR".to_string())
        .or_insert_with(|| agent_dir_path.to_string_lossy().into_owned());
    opts.env
        .entry("HIVE_ROOT".to_string())
        .or_insert_with(|| hive_root.to_string_lossy().into_owned());
    opts.env
        .entry("MEMPALACE_PALACE_PATH".to_string())
        .or_insert_with(|| home.join("palace").to_string_lossy().into_owned());
    opts.env
        .entry("MD_SERVER_URL".to_string())
        .or_insert_with(|| format!("http://127.0.0.1:{port}"));
    opts.env
        .entry("MD_HOOK_URL".to_string())
        .or_insert_with(|| format!("http://127.0.0.1:{port}/hook/{agent_id}"));
    opts.env
        .entry("MD_HOOK_TOKEN".to_string())
        .or_insert_with(|| state.hooks.http_token.clone());

    let broker_url = format!("http://127.0.0.1:{port}/broker");
    let broker_tok = {
        let mut broker = state.broker.lock().unwrap();
        let allowed_ids: Vec<String> = broker
            .list_records()
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| r.id.clone())
            .collect();
        broker.grant_capability(agent_id, allowed_ids, true)
    };
    opts.env
        .entry("MD_BROKER_URL".to_string())
        .or_insert_with(|| broker_url.clone());
    opts.env
        .entry("MD_BROKER_TOKEN".to_string())
        .or_insert_with(|| broker_tok.clone());
    let broker_file = agent_dir_path.join(".broker");
    if let Ok(payload) = serde_json::to_string(&serde_json::json!({
        "url": broker_url,
        "token": broker_tok,
    })) && atomic_write_string(&broker_file, &payload).is_ok()
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&broker_file, fs::Permissions::from_mode(0o600));
        }
    }
    let custom_env = state.secrets.lock().unwrap().custom_secret_env(&opts.env);
    for (k, v) in custom_env {
        opts.env.entry(k).or_insert(v);
    }
    let cur_path = opts
        .env
        .get("PATH")
        .cloned()
        .unwrap_or_else(|| ensure_extra_path_entries(std::env::var("PATH").ok()));
    let bin_str = hive_root.join("bin").to_string_lossy().into_owned();
    if !cur_path.split(':').any(|p| p == bin_str) {
        opts.env
            .insert("PATH".to_string(), format!("{bin_str}:{cur_path}"));
    } else {
        opts.env.entry("PATH".to_string()).or_insert(cur_path);
    }

    // Apply `maxTurns` and OpenCode non-interactive config/plugin bridge.
    let (max_turns, auto_mode) = {
        let mut store = state.config.lock().unwrap();
        let cfg = store.read();
        let mt = cfg
            .extra
            .get("maxTurns")
            .and_then(|v| v.as_u64())
            .filter(|&n| n > 0);
        let am = cfg
            .extra
            .get("autoMode")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        (mt, am)
    };
    if provider == "claude"
        && !opts.args.iter().any(|a| a == "--max-turns")
        && let Some(n) = max_turns
    {
        opts.args.push("--max-turns".to_string());
        opts.args.push(n.to_string());
    }

    if is_safe_id(agent_id) {
        let agent_dir = hive_root.join("agents").join(agent_id);
        match provider {
            "claude" => {
                if !opts.args.iter().any(|a| a == "--settings")
                    && let Some(settings_path) = install_claude_hooks_settings(&agent_dir)
                {
                    opts.args.push("--settings".to_string());
                    opts.args.push(settings_path.to_string_lossy().into_owned());
                }
            }
            "codex" => {
                if !opts.env.contains_key("CODEX_HOME")
                    && let Some(codex_home) = install_codex_home(&home, &hive_root, &agent_dir)
                {
                    opts.env.insert(
                        "CODEX_HOME".to_string(),
                        codex_home.to_string_lossy().into_owned(),
                    );
                }
            }
            "opencode" => {
                if !opts.args.iter().any(|a| a == "--standalone") {
                    opts.args.push("--standalone".to_string());
                }
                if !opts.env.contains_key("OPENCODE_CONFIG_CONTENT")
                    && let Some(content) =
                        install_opencode_config_content(&agent_dir, &hive_root, auto_mode)
                {
                    opts.env
                        .insert("OPENCODE_CONFIG_CONTENT".to_string(), content);
                }
            }
            "gemini" => {
                if !opts.env.contains_key("GEMINI_CLI_SYSTEM_SETTINGS_PATH")
                    && let Some(settings_path) = install_gemini_hooks(&agent_dir)
                {
                    opts.env.insert(
                        "GEMINI_CLI_SYSTEM_SETTINGS_PATH".to_string(),
                        settings_path.to_string_lossy().into_owned(),
                    );
                }
            }
            "pi" => {
                if !opts.env.contains_key("PI_CODING_AGENT_DIR")
                    && let Some(pi_dir) = install_pi_hooks(&agent_dir)
                {
                    opts.env.insert(
                        "PI_CODING_AGENT_DIR".to_string(),
                        pi_dir.to_string_lossy().into_owned(),
                    );
                }
            }
            _ => {}
        }
    }

    sync_fleet_json(state);
    Some(build_seed_prompt(
        &hive_root,
        agent_id,
        name,
        is_god,
        role,
        &opts.cwd,
    ))
}

pub fn build_hook_context_for_agent(
    state: &AppState,
    agent_id: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    state.sync_hive_home();
    let cfg = state.config.lock().unwrap().read();
    let response_style = cfg
        .extra
        .get("responseStyle")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let reg = state.hive.lock().unwrap().registry();
    let agents = reg.get("agents").and_then(|v| v.as_object());
    let agent = agents.and_then(|m| m.get(agent_id)).and_then(|v| v.as_object());
    let standing_goal = agent
        .and_then(|a| a.get("goal").or_else(|| a.get("role")))
        .and_then(|v| v.as_str())
        .map(|g| format!("<goal>\n{g}\n</goal>"));
    let hive_root = state
        .harness_home()
        .map(|h| h.join("hive").to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = agent
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(agent_id);
    let identity = if hive_root.is_empty() {
        Some(format!("AGENT_ID={agent_id}, AGENT_NAME={name}"))
    } else {
        Some(format!(
            "AGENT_ID={agent_id}, AGENT_NAME={name}, HIVE_ROOT={hive_root}"
        ))
    };
    let ptys = state.pty.list();
    let contexts = state.hooks.context_by_id.lock().unwrap().clone();
    let fleet_path = if hive_root.is_empty() {
        "fleet.json".to_string()
    } else {
        format!("{hive_root}/fleet.json")
    };
    let mut rows: Vec<String> = Vec::new();
    if let Some(m) = agents {
        let mut keys: Vec<&String> = m.keys().collect();
        keys.sort();
        for id in keys {
            let v = &m[id];
            if v.get("archived").and_then(|x| x.as_bool()).unwrap_or(false) {
                continue;
            }
            let sleeping = v.get("sleeping").and_then(|x| x.as_bool()).unwrap_or(false);
            let pty_live = ptys.iter().any(|p| p.id == format!("pty-{id}"));
            if !pty_live && !sleeping {
                continue;
            }
            let pname = v.get("name").and_then(|x| x.as_str()).unwrap_or(id);
            let role = v.get("role").and_then(|x| x.as_str()).unwrap_or("agent");
            let mut bits = vec![role.to_string()];
            if sleeping {
                bits.push("Sleeping".to_string());
            }
            if v.get("onHold").and_then(|x| x.as_bool()).unwrap_or(false) {
                bits.push("ON HOLD — 1:1 with the human".to_string());
            }
            if id == agent_id {
                bits.push("you".to_string());
            }
            if let Some(cw) = contexts.get(id)
                && cw.limit > 0
            {
                let pct = ((cw.tokens as f64 / cw.limit as f64) * 100.0).round() as u64;
                bits.push(format!("ctx {}%", pct.min(100)));
            }
            rows.push(format!("{id} \"{pname}\" ({})", bits.join(", ")));
        }
    }
    let roster_ctx = if rows.is_empty() {
        None
    } else {
        Some(format!(
            "[LIVE ROSTER — auto-injected from {fleet_path}] {} ACTIVE agent(s): {}. This is the CURRENT floor and it SUPERSEDES any roster earlier in this conversation — agents you remember that are absent here have been archived or stopped, so do not message them. Route work to someone on this list before spawning anyone new; when hiring temps for parallel/unmatched work, write $HIVE_ROOT/spawn-requests/<id>.json (never use CLI-internal Task/Agent subagent tools).",
            rows.len(),
            rows.join("; ")
        ))
    };
    (standing_goal, response_style, identity, roster_ctx)
}

pub fn ensure_extra_path_entries(existing: Option<String>) -> String {
    let mut seen = HashSet::new();
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut push_dir = |p: PathBuf| {
        let key = p.to_string_lossy().into_owned();
        if !key.is_empty() && seen.insert(key) {
            parts.push(p);
        }
    };
    if let Some(cur) = existing {
        for p in std::env::split_paths(&cur) {
            push_dir(p);
        }
    }
    #[cfg(not(windows))]
    for sys in [
        "/opt/homebrew/bin",
        "/home/linuxbrew/.linuxbrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
    ] {
        push_dir(PathBuf::from(sys));
    }
    if let Some(home) = dirs::home_dir() {
        for rel in [".local/bin", ".cargo/bin", ".bun/bin", ".npm-global/bin", ".grok/bin"] {
            push_dir(home.join(rel));
        }
    }
    std::env::join_paths(parts)
        .map(|os| os.to_string_lossy().into_owned())
        .unwrap_or_default()
}
