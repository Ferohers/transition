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
Every agent runs with environment variables: `AGENT_ID`, `AGENT_NAME`, `AGENT_DIR`, `HIVE_ROOT`, `MEMPALACE_PALACE_PATH`, `MD_SERVER_URL`, `MD_HOOK_URL`, `MD_HOOK_TOKEN`.

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
- Note: Respects Settings -> Autonomy & Budgets (`orchestratorMaySpawn` & `maxConcurrentWorkers`).

## 3. Fleet Status, Board, Tasks & Memory
- **Fleet status**: `$HIVE_ROOT/fleet.json` (live per-agent status, breaker state, tokens, inbox backlog) and `$HIVE_ROOT/registry.json`.
- **Shared board**: `$HIVE_ROOT/board.md` (`god` acts as scribe).
- **Kanban tasks**: `$HIVE_ROOT/tasks.json` (`{{"tasks":[{{"id":"...","title":"...","status":"todo|doing|done|blocked","assignee":"...","humanQA":[]}}]}}`).
- **Memory & Index**: `$HIVE_ROOT/agents/$AGENT_ID/memory.md` and `$HIVE_ROOT/memory-index.md`.
- Full protocol rules: `$HIVE_ROOT/PROTOCOL.md`.
"#,
    );

    let path = hive_root.join("COMMANDS.md");
    if fs::read_to_string(&path).ok().as_deref() != Some(commands_content.as_str()) {
        let _ = atomic_write_string(&path, &commands_content);
    }
}

pub fn ensure_agent_hive_dirs(hive_root: &Path, agent_id: &str, name: &str) {
    let _ = fs::create_dir_all(hive_root.join("spawn-requests"));
    let _ = fs::create_dir_all(hive_root.join("compact-requests"));
    let _ = fs::create_dir_all(hive_root.join("clear-requests"));
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
    let breakers = state.breaker.lock().unwrap();
    let ptys = state.pty.list();

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
            let name = info.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            if !archived {
                ensure_agent_hive_dirs(&hive_root, id, name);
            }
            let pty_running = ptys.iter().any(|p| p.id == format!("pty-{id}"));
            let stopped = info
                .get("stopped")
                .and_then(|v| v.as_bool())
                .unwrap_or(!pty_running);
            let ctx = contexts.get(id);
            let inbox_backlog = state.hive.lock().unwrap().inbox(id).len();
            rows.push(json!({
                "id": id,
                "name": name,
                "role": info.get("role").and_then(|v| v.as_str()).unwrap_or(""),
                "provider": info.get("provider").and_then(|v| v.as_str()).unwrap_or("claude"),
                "model": models.get(id).cloned().or_else(|| info.get("model").and_then(|v| v.as_str()).map(ToString::to_string)),
                "status": info.get("status").and_then(|v| v.as_str()).unwrap_or("idle"),
                "sleeping": info.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false),
                "stopped": stopped,
                "archived": archived,
                "isGod": is_god,
                "isTemp": is_temp,
                "cwd": info.get("cwd").cloned().unwrap_or(Value::Null),
                "breaker": breakers.state_for(id).level,
                "contextTokens": ctx.map(|c| c.tokens),
                "contextLimit": ctx.map(|c| c.limit),
                "inboxBacklog": inbox_backlog,
            }));
        }
    }

    let doc = json!({
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
    let plugin_path = agent_dir.join("opencode-hive-plugin.mjs");
    let plugin_src = r#"async function postHook(payload) {
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

export const HiveBridge = async () => ({
  event: async (input) => {
    try {
      if (input?.event?.type === "session.idle") {
        await postHook({ hook_event_name: "Stop" });
      }
    } catch {}
  },
  "tool.execute.before": async (input) => {
    try {
      await postHook({
        hook_event_name: "PreToolUse",
        tool_name: input?.tool || input?.name || "tool",
        tool_input: input?.args || {},
      });
    } catch {}
  },
  "tool.execute.after": async (input) => {
    try {
      await postHook({
        hook_event_name: "PostToolUse",
        tool_name: input?.tool || input?.name || "tool",
        tool_input: input?.args || {},
      });
    } catch {}
  },
  "chat.message": async (_input, output) => {
    try {
      const parts = Array.isArray(output?.parts) ? output.parts : [];
      const text = parts
        .filter((p) => p && p.type === "text" && typeof p.text === "string" && !p.synthetic && !p.ignored)
        .map((p) => p.text)
        .join("\n");
      if (text) {
        await postHook({ hook_event_name: "UserPromptSubmit", prompt: text.slice(-4096) });
      }
    } catch {}
  },
});
export default HiveBridge;
"#;
    let _ = atomic_write_string(&plugin_path, plugin_src);

    let mut cfg = Map::new();
    cfg.insert("autoupdate".to_string(), Value::Bool(false));
    let mut plugin_url = String::from("file://");
    plugin_url.push_str(&plugin_path.to_string_lossy());
    cfg.insert("plugin".to_string(), json!([plugin_url]));
    cfg.insert(
        "instructions".to_string(),
        json!([
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
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_cmd }] }],
        }
    });
    if let Ok(text) = serde_json::to_string_pretty(&hooks_doc) {
        let _ = atomic_write_string(&codex_dir.join("hooks.json"), &text);
    }
    Some(codex_dir)
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
        if let Some(r) = role.filter(|s| !s.trim().is_empty()) {
            entry.insert("role".to_string(), Value::String(r.to_string()));
        }
        let _ = state.hive.lock().unwrap().register_worker(agent_id, entry);
    }

    let _ = md_hive::memory::write_memory_index(&home);

    let port = state.port;
    let agent_dir_path = hive_root.join("agents").join(agent_id);
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
    if !opts.env.contains_key("PATH") {
        opts.env.insert(
            "PATH".to_string(),
            ensure_extra_path_entries(std::env::var("PATH").ok()),
        );
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
                if !opts.env.contains_key("OPENCODE_CONFIG_CONTENT")
                    && let Some(content) =
                        install_opencode_config_content(&agent_dir, &hive_root, auto_mode)
                {
                    opts.env
                        .insert("OPENCODE_CONFIG_CONTENT".to_string(), content);
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
    let peers: Vec<String> = agents
        .map(|m| {
            m.iter()
                .filter(|(id, v)| {
                    *id != agent_id
                        && !v.get("archived").and_then(|x| x.as_bool()).unwrap_or(false)
                })
                .map(|(id, v)| {
                    let pname = v.get("name").and_then(|x| x.as_str()).unwrap_or(id);
                    format!("{id} ({pname})")
                })
                .collect()
        })
        .unwrap_or_default();
    let roster_ctx = if peers.is_empty() {
        None
    } else {
        Some(format!("Active teammates: {}", peers.join(", ")))
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
