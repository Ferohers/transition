//! Hive filesystem manager (`registry.json`, `tasks.json`, `board.md`,
//! `log.jsonl`, `agents/<id>/inbox`) mirroring `src/main/hive.ts`.

use md_core::CoreError;
use md_core::damaged::{DamagedFiles, IntegrityIssue, JsonShape, ReadOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveSearchHit {
    pub file: String,
    pub line: usize,
    pub snippet: String,
}

#[derive(Debug, Default)]
pub struct HiveManager {
    home: Option<PathBuf>,
    damage: DamagedFiles,
}

impl HiveManager {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            damage: DamagedFiles::new(),
        }
    }

    pub fn set_home(&mut self, home: Option<PathBuf>) {
        self.home = home;
    }

    pub fn root(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|h| h.join("hive"))
    }

    pub fn ensure_hive(&self) -> Result<PathBuf, CoreError> {
        let root = self
            .root()
            .ok_or_else(|| CoreError::InvalidPath("no harnessHome configured".to_string()))?;
        fs::create_dir_all(root.join("agents"))?;
        fs::create_dir_all(root.join("projects"))?;
        fs::create_dir_all(root.join("bin"))?;
        let board = root.join("board.md");
        if !board.exists() {
            fs::write(
                board,
                "# Hive board\n\n_Shared plans live here. The god agent is the scribe._\n",
            )?;
        }
        ensure_hive_gitignore(&root);
        Ok(root)
    }

    pub fn integrity_issues(&mut self) -> Vec<IntegrityIssue> {
        if let Some(root) = self.root() {
            let _ = self.damage.read(&root.join("registry.json"), JsonShape::Registry);
            let _ = self.damage.read(&root.join("tasks.json"), JsonShape::Tasks);
        }
        self.damage.list()
    }

    pub fn registry(&mut self) -> Value {
        let Some(root) = self.root() else {
            return serde_json::json!({ "agents": {} });
        };
        let path = root.join("registry.json");
        match self.damage.read(&path, JsonShape::Registry) {
            ReadOutcome::Ok(v) => v,
            _ => serde_json::json!({ "agents": {} }),
        }
    }

    fn write_registry(&mut self, reg: &Value) -> Result<(), CoreError> {
        let root = self.ensure_hive()?;
        let path = root.join("registry.json");
        if self.damage.is_damaged(&path, JsonShape::Registry) {
            return Err(CoreError::InvalidPath(
                "registry.json is damaged; refusing to overwrite".to_string(),
            ));
        }
        atomic_write_json(&path, reg)?;
        self.damage.mark_wrote(&path);
        Ok(())
    }

    pub fn sync_roster(&mut self, agents: &[Value], archived: &[Value]) -> Result<(), CoreError> {
        if self.root().is_none() {
            return Ok(());
        }
        let mut reg = self.registry();
        if !reg.is_object() {
            reg = serde_json::json!({ "agents": {} });
        }
        let reg_obj = reg.as_object_mut().unwrap();
        if !reg_obj.get("agents").map(|v| v.is_object()).unwrap_or(false) {
            reg_obj.insert("agents".to_string(), Value::Object(Map::new()));
        }
        let agents_map = reg_obj
            .get_mut("agents")
            .and_then(|v| v.as_object_mut())
            .unwrap();

        let mut changed = false;
        let mut present: HashSet<String> = HashSet::new();
        for (list, is_archived) in [(agents, false), (archived, true)] {
            for item in list {
                let Some(obj) = item.as_object() else { continue };
                let Some(id) = obj.get("id").and_then(|v| v.as_str()).filter(|s| is_safe_id(s)) else {
                    continue;
                };
                present.insert(id.to_string());
                let is_god = obj.get("isGod").and_then(|v| v.as_bool()).unwrap_or(false)
                    || id == "god";
                if let Some(existing) = agents_map.get_mut(id).and_then(|v| v.as_object_mut()) {
                    // Preserve backend hook-driven `status` once present so lagged UI roster saves
                    // cannot clobber `Stop` -> `idle` transitions back to `working`.
                    for k in ["name", "provider", "cwd", "isGod", "sleeping", "onHold"] {
                        if let Some(v) = obj.get(k).cloned()
                            && existing.get(k) != Some(&v)
                        {
                            existing.insert(k.to_string(), v);
                            changed = true;
                        }
                    }
                    if !existing.contains_key("status")
                        && let Some(v) = obj.get("status").cloned()
                    {
                        existing.insert("status".to_string(), v);
                        changed = true;
                    }
                    if is_god {
                        let desc = obj.get("description").and_then(|v| v.as_str());
                        let existing_role = existing.get("role").and_then(|v| v.as_str());
                        if existing_role.is_none_or(|s| s.trim().is_empty() || Some(s) == desc) {
                            existing.insert(
                                "role".to_string(),
                                Value::String("orchestrator (god)".to_string()),
                            );
                            changed = true;
                        }
                    }
                    if existing.get("archived").and_then(|v| v.as_bool()) != Some(is_archived) {
                        existing.insert("archived".to_string(), Value::Bool(is_archived));
                        changed = true;
                    }
                    continue;
                }
                let mut entry = Map::new();
                entry.insert("id".to_string(), Value::String(id.to_string()));
                if let Some(name) = obj.get("name").cloned() {
                    entry.insert("name".to_string(), name);
                }
                if is_god {
                    let role_val = obj
                        .get("role")
                        .filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
                        .cloned()
                        .unwrap_or_else(|| Value::String("orchestrator (god)".to_string()));
                    entry.insert("role".to_string(), role_val);
                } else if let Some(role) = obj.get("role").or_else(|| obj.get("description")).cloned()
                {
                    entry.insert("role".to_string(), role);
                }
                for k in ["provider", "cwd", "isGod", "sleeping", "status"] {
                    if let Some(v) = obj.get(k).cloned() {
                        entry.insert(k.to_string(), v);
                    }
                }
                entry.insert("archived".to_string(), Value::Bool(is_archived));
                agents_map.insert(id.to_string(), Value::Object(entry));
                if !is_archived && let Some(root) = self.root() {
                    let adir = root.join("agents").join(id);
                    for sub in ["inbox", "inbox/.done", "outbox", "outbox/.sent", "spawn-requests"] {
                        let _ = fs::create_dir_all(adir.join(sub));
                    }
                }
                changed = true;
            }
        }
        // Mark removed non-temp agents as archived & stopped so registry/fleet stay accurate.
        for (id, val) in agents_map.iter_mut() {
            if present.contains(id) {
                continue;
            }
            let Some(obj) = val.as_object_mut() else {
                continue;
            };
            let is_temp = obj.get("isTemp").and_then(|v| v.as_bool()).unwrap_or(false)
                || id.starts_with("worker-");
            let is_god = obj.get("isGod").and_then(|v| v.as_bool()).unwrap_or(false)
                || id == "god";
            if is_temp || is_god {
                continue;
            }
            if obj.get("archived").and_then(|v| v.as_bool()) != Some(true) {
                obj.insert("archived".to_string(), Value::Bool(true));
                obj.insert("stopped".to_string(), Value::Bool(true));
                changed = true;
            }
        }
        if changed {
            self.write_registry(&reg)?;
        }
        Ok(())
    }

    pub fn patch_agent_field(
        &mut self,
        id: &str,
        key: &str,
        value: Option<Value>,
    ) -> Result<bool, CoreError> {
        let mut reg = self.registry();
        let Some(agents) = reg.get_mut("agents").and_then(|v| v.as_object_mut()) else {
            return Ok(false);
        };
        let Some(agent) = agents.get_mut(id).and_then(|v| v.as_object_mut()) else {
            return Ok(false);
        };
        match value {
            Some(v) => {
                agent.insert(key.to_string(), v);
            }
            None => {
                agent.remove(key);
            }
        }
        self.write_registry(&reg)?;
        Ok(true)
    }

    pub fn tasks(&mut self) -> Value {
        let Some(root) = self.root() else {
            return serde_json::json!({ "tasks": [] });
        };
        let path = root.join("tasks.json");
        match self.damage.read(&path, JsonShape::Tasks) {
            ReadOutcome::Ok(mut v) => {
                if self.auto_key_tasks(&root, &mut v) > 0
                    && atomic_write_json(&path, &v).is_ok()
                {
                    self.damage.mark_wrote(&path);
                }
                v
            }
            _ => serde_json::json!({ "tasks": [] }),
        }
    }

    fn default_ticket_prefix(&self, root: &Path) -> String {
        let base = root
            .parent()
            .and_then(|h| h.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let mut letters: String = base
            .split(|c: char| !c.is_alphanumeric())
            .filter_map(|part| part.chars().next())
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if letters.len() < 2 {
            letters = base
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(3)
                .map(|c| c.to_ascii_uppercase())
                .collect();
        }
        if letters.len() >= 2 {
            letters.truncate(4);
            letters
        } else {
            "MD".to_string()
        }
    }

    fn auto_key_tasks(&self, root: &Path, ledger: &mut Value) -> usize {
        let fallback_prefix = self.default_ticket_prefix(root);
        let Some(obj) = ledger.as_object_mut() else {
            return 0;
        };
        let Some(tasks_arr) = obj.get("tasks").and_then(|v| v.as_array()) else {
            return 0;
        };
        let needs_key = tasks_arr.iter().any(|t| {
            t.get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        });
        if !needs_key {
            return 0;
        }
        let mut prefix = obj
            .get("ticket")
            .and_then(|t| t.get("prefix"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&fallback_prefix)
            .to_string();
        if prefix.is_empty() {
            prefix = "MD".to_string();
        }
        let mut next_num = obj
            .get("ticket")
            .and_then(|t| t.get("next"))
            .and_then(|v| v.as_u64())
            .filter(|&n| n >= 1)
            .unwrap_or(1);
        let existing_ids: std::collections::HashSet<String> = tasks_arr
            .iter()
            .filter_map(|t| t.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
            .collect();
        let mut assigned = 0usize;
        if let Some(list) = obj.get_mut("tasks").and_then(|v| v.as_array_mut()) {
            for card in list {
                let Some(card_obj) = card.as_object_mut() else {
                    continue;
                };
                let missing = card_obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty();
                if missing {
                    let mut candidate = format!("{prefix}-{next_num}");
                    while existing_ids.contains(&candidate) {
                        next_num += 1;
                        candidate = format!("{prefix}-{next_num}");
                    }
                    card_obj.insert("id".to_string(), Value::String(candidate));
                    next_num += 1;
                    assigned += 1;
                }
            }
        }
        if assigned > 0 {
            let ticket = obj
                .entry("ticket".to_string())
                .or_insert_with(|| serde_json::json!({}));
            if let Some(t_obj) = ticket.as_object_mut() {
                t_obj.insert("prefix".to_string(), Value::String(prefix));
                t_obj.insert("next".to_string(), Value::Number(next_num.into()));
            }
            self.append_log(&serde_json::json!({
                "kind": "tasks-keyed",
                "count": assigned,
            }));
        }
        assigned
    }

    pub fn tasks_archive(&self) -> Value {
        let Some(root) = self.root() else {
            return serde_json::json!({ "tasks": [] });
        };
        let path = root.join("tasks-archive.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({ "tasks": [] }))
    }

    pub fn add_task(&mut self, task: Value) -> Result<bool, CoreError> {
        let root = self.ensure_hive()?;
        let path = root.join("tasks.json");
        let mut ledger = self.read_tasks_for_write(&path)?;
        let list = ensure_tasks_array(&mut ledger);
        if let Some(id) = task.get("id").and_then(|v| v.as_str())
            && list
                .iter()
                .any(|existing| existing.get("id").and_then(|v| v.as_str()) == Some(id))
        {
            return Ok(false);
        }
        list.push(task);
        atomic_write_json(&path, &ledger)?;
        self.damage.mark_wrote(&path);
        Ok(true)
    }

    pub fn patch_task(&mut self, id: &str, patch: &Map<String, Value>) -> Result<bool, CoreError> {
        let root = self.ensure_hive()?;
        let path = root.join("tasks.json");
        let mut ledger = self.read_tasks_for_write(&path)?;
        let list = ensure_tasks_array(&mut ledger);
        let Some(card) = list
            .iter_mut()
            .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|v| v.as_object_mut())
        else {
            return Ok(false);
        };
        for (k, v) in patch {
            if k != "id" {
                card.insert(k.clone(), v.clone());
            }
        }
        atomic_write_json(&path, &ledger)?;
        self.damage.mark_wrote(&path);
        Ok(true)
    }

    pub fn delete_task(&mut self, id: &str) -> Result<bool, CoreError> {
        let root = self.ensure_hive()?;
        let path = root.join("tasks.json");
        let mut ledger = self.read_tasks_for_write(&path)?;
        let list = ensure_tasks_array(&mut ledger);
        let before = list.len();
        list.retain(|c| c.get("id").and_then(|v| v.as_str()) != Some(id));
        if list.len() == before {
            return Ok(false);
        }
        atomic_write_json(&path, &ledger)?;
        self.damage.mark_wrote(&path);
        Ok(true)
    }

    fn read_tasks_for_write(&mut self, path: &Path) -> Result<Value, CoreError> {
        match self.damage.read(path, JsonShape::Tasks) {
            ReadOutcome::Ok(v) => Ok(v),
            ReadOutcome::Missing => Ok(serde_json::json!({ "tasks": [] })),
            ReadOutcome::Damaged | ReadOutcome::Unreadable => Err(CoreError::InvalidPath(
                "tasks.json is unreadable or damaged; refusing to overwrite".to_string(),
            )),
        }
    }

    pub fn board(&self) -> String {
        let Some(root) = self.root() else {
            return String::new();
        };
        fs::read_to_string(root.join("board.md")).unwrap_or_default()
    }

    pub fn append_log(&self, entry: &Value) {
        self.append_log_with_cap(entry, 10 * 1024 * 1024);
    }

    fn append_log_with_cap(&self, entry: &Value, max_bytes: u64) {
        let Ok(root) = self.ensure_hive() else {
            return;
        };
        let path = root.join("log.jsonl");
        if max_bytes > 0
            && let Ok(meta) = fs::metadata(&path)
            && meta.len() >= max_bytes
        {
            let backup = root.join("log.jsonl.1");
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(&path, &backup);
        }
        let mut obj = entry.as_object().cloned().unwrap_or_default();
        if !obj.contains_key("ts") {
            obj.insert("ts".to_string(), Value::from(chrono::Utc::now().to_rfc3339()));
        }
        if let Ok(line) = serde_json::to_string(&Value::Object(obj))
            && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn log_tail(&self, limit: usize) -> Vec<Value> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        let Ok(raw) = fs::read_to_string(root.join("log.jsonl")) else {
            return Vec::new();
        };
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = lines.len().saturating_sub(limit.clamp(1, 1000));
        lines[start..]
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect()
    }

    pub fn inbox(&self, agent_id: &str) -> Vec<Value> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        if !is_safe_id(agent_id) {
            return Vec::new();
        }
        read_json_dir(&root.join("agents").join(agent_id).join("inbox"))
    }

    /// Move handled inbox files (`<id>.json`) to `agents/<agent_id>/inbox/.done/`.
    pub fn file_inbox_messages(&self, agent_id: &str, ids: &[String]) -> usize {
        let Some(root) = self.root() else {
            return 0;
        };
        if !is_safe_id(agent_id) || ids.is_empty() {
            return 0;
        }
        let inbox_dir = root.join("agents").join(agent_id).join("inbox");
        let done_dir = inbox_dir.join(".done");
        let _ = fs::create_dir_all(&done_dir);
        let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let Ok(entries) = fs::read_dir(&inbox_dir) else {
            return 0;
        };
        let mut moved = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let matches_id = wanted.contains(stem)
                || fs::read_to_string(&path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| wanted.contains(s)))
                    .unwrap_or(false);
            if matches_id
                && let Some(fname) = path.file_name()
                && fs::rename(&path, done_dir.join(fname)).is_ok()
            {
                moved += 1;
            }
        }
        moved
    }

    pub fn send_message(&mut self, mut msg: Value, from: &str) -> Result<Value, CoreError> {
        let root = self.ensure_hive()?;
        let Some(obj) = msg.as_object_mut() else {
            return Err(CoreError::InvalidPath("message must be a JSON object".to_string()));
        };
        let to = obj
            .get("to")
            .and_then(|v| v.as_str())
            .filter(|s| is_safe_id(s))
            .ok_or_else(|| CoreError::InvalidPath("invalid or missing recipient 'to'".to_string()))?
            .to_string();
        let id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| is_safe_id(s))
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("msg-{}", chrono::Utc::now().timestamp_millis()));
        let ts = obj
            .get("ts")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let conversation = obj
            .get("conversation")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| id.clone());
        let act = obj
            .get("act")
            .and_then(|v| v.as_str())
            .unwrap_or("inform")
            .to_string();
        let subject = obj
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        obj.insert("id".to_string(), Value::String(id.clone()));
        obj.insert("from".to_string(), Value::String(from.to_string()));
        obj.insert("to".to_string(), Value::String(to.clone()));
        obj.insert("ts".to_string(), Value::from(ts));
        obj.insert("conversation".to_string(), Value::String(conversation));
        obj.insert("act".to_string(), Value::String(act.clone()));
        obj.insert(
            "targets".to_string(),
            Value::Array(vec![Value::String(to.clone())]),
        );

        let inbox_dir = root.join("agents").join(&to).join("inbox");
        fs::create_dir_all(&inbox_dir)?;
        atomic_write_json(&inbox_dir.join(format!("{id}.json")), &Value::Object(obj.clone()))?;

        self.append_log(&serde_json::json!({
            "kind": "message",
            "id": id,
            "from": from,
            "to": to,
            "act": act,
            "subject": subject,
            "ts": ts,
        }));
        Ok(Value::Object(obj.clone()))
    }

    /// Drain all `agents/<id>/outbox/*.json` files into recipient inboxes and
    /// return the delivered message objects (plus `targets` array for broadcast).
    pub fn drain_outboxes(&mut self) -> Vec<Value> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        let agents_dir = root.join("agents");
        let Ok(agent_dirs) = fs::read_dir(&agents_dir) else {
            return Vec::new();
        };

        let mut active_ids: Vec<String> = Vec::new();
        if let Some(map) = self.registry().get("agents").and_then(|v| v.as_object()) {
            for (aid, info) in map {
                let archived = info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false);
                if !archived && is_safe_id(aid) {
                    active_ids.push(aid.clone());
                }
            }
        }

        let mut delivered = Vec::new();
        for dir_entry in agent_dirs.flatten() {
            let from_id = dir_entry.file_name().to_string_lossy().into_owned();
            if !is_safe_id(&from_id) {
                continue;
            }
            let outbox = dir_entry.path().join("outbox");
            let Ok(files) = fs::read_dir(&outbox) else {
                continue;
            };
            let sent_dir = outbox.join(".sent");
            let mut json_files: Vec<PathBuf> = files
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json"))
                .collect();
            json_files.sort();

            for path in json_files {
                let fname = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "msg.json".to_string());
                let raw = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let Ok(mut val) = serde_json::from_str::<Value>(&raw) else {
                    let _ = fs::create_dir_all(&sent_dir);
                    let _ = fs::rename(&path, sent_dir.join(format!("bad-{fname}")));
                    continue;
                };
                let Some(obj) = val.as_object_mut() else {
                    let _ = fs::create_dir_all(&sent_dir);
                    let _ = fs::rename(&path, sent_dir.join(format!("bad-{fname}")));
                    continue;
                };
                let to_raw = obj
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let targets: Vec<String> = if to_raw == "broadcast" {
                    active_ids
                        .iter()
                        .filter(|id| *id != &from_id)
                        .cloned()
                        .collect()
                } else if to_raw == "human" {
                    obj.entry("needsHuman".to_string())
                        .or_insert(Value::Bool(true));
                    if from_id == "god" {
                        Vec::new()
                    } else {
                        vec!["god".to_string()]
                    }
                } else if is_safe_id(&to_raw) {
                    if to_raw == from_id {
                        Vec::new()
                    } else {
                        vec![to_raw.clone()]
                    }
                } else {
                    Vec::new()
                };

                if targets.is_empty() && !matches!(to_raw.as_str(), "broadcast" | "human") {
                    let _ = fs::create_dir_all(&sent_dir);
                    let _ = fs::rename(&path, sent_dir.join(format!("bad-{fname}")));
                    continue;
                }

                let id = format!(
                    "out-{}-{}",
                    chrono::Utc::now().timestamp_millis(),
                    fname.trim_end_matches(".json")
                );
                let ts = chrono::Utc::now().timestamp_millis();
                let act = obj
                    .get("act")
                    .and_then(|v| v.as_str())
                    .unwrap_or("inform")
                    .to_string();
                let subject = obj
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let conversation = obj
                    .get("conversation")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| id.clone());

                obj.insert("id".to_string(), Value::String(id.clone()));
                obj.insert("from".to_string(), Value::String(from_id.clone()));
                obj.insert("to".to_string(), Value::String(to_raw.clone()));
                obj.insert("ts".to_string(), Value::from(ts));
                obj.insert("act".to_string(), Value::String(act.clone()));
                obj.insert("conversation".to_string(), Value::String(conversation));
                obj.insert(
                    "targets".to_string(),
                    Value::Array(targets.iter().map(|t| Value::String(t.clone())).collect()),
                );

                let _ = fs::create_dir_all(&sent_dir);
                if fs::rename(&path, sent_dir.join(&fname)).is_err() {
                    continue;
                }

                for target in &targets {
                    let inbox_dir = agents_dir.join(target).join("inbox");
                    if fs::create_dir_all(&inbox_dir).is_ok() {
                        let _ = atomic_write_json(&inbox_dir.join(format!("{id}.json")), &val);
                    }
                }
                self.append_log(&serde_json::json!({
                    "kind": "message",
                    "id": id,
                    "from": from_id,
                    "to": to_raw,
                    "targets": targets,
                    "act": act,
                    "subject": subject,
                    "ts": ts,
                }));
                delivered.push(val);
            }
        }
        delivered
    }

    pub fn register_worker(&mut self, id: &str, entry: Map<String, Value>) -> Result<(), CoreError> {
        let root = self.ensure_hive()?;
        if is_safe_id(id) {
            let adir = root.join("agents").join(id);
            for sub in ["inbox", "inbox/.done", "outbox", "outbox/.sent", "spawn-requests"] {
                let _ = fs::create_dir_all(adir.join(sub));
            }
        }
        let mut reg = self.registry();
        if !reg.is_object() {
            reg = serde_json::json!({ "agents": {} });
        }
        let reg_obj = reg.as_object_mut().unwrap();
        if !reg_obj.get("agents").map(|v| v.is_object()).unwrap_or(false) {
            reg_obj.insert("agents".to_string(), Value::Object(Map::new()));
        }
        if let Some(agents_map) = reg_obj.get_mut("agents").and_then(|v| v.as_object_mut()) {
            if let Some(existing) = agents_map.get_mut(id).and_then(|v| v.as_object_mut()) {
                for (k, v) in entry {
                    existing.insert(k, v);
                }
            } else {
                agents_map.insert(id.to_string(), Value::Object(entry));
            }
        }
        self.write_registry(&reg)
    }

    pub fn messages(&self, agent_id: Option<&str>, limit: usize) -> Vec<Value> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        let agents_dir = root.join("agents");
        let Ok(dirs) = fs::read_dir(&agents_dir) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for entry in dirs.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(target) = agent_id
                && name != target
            {
                continue;
            }
            for sub in ["inbox", "inbox/.done", "outbox", "outbox/.sent"] {
                for msg in read_json_dir(&entry.path().join(sub)) {
                    let id = msg
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("{name}:{sub}:{}", out.len()));
                    if seen.insert(id) {
                        out.push(msg);
                    }
                }
            }
        }
        out.truncate(limit.clamp(1, 200));
        out
    }

    pub fn text_search(&self, query: &str) -> Vec<HiveSearchHit> {
        let Some(root) = self.root() else {
            return Vec::new();
        };
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for name in ["board.md", "tasks.json", "registry.json"] {
            let path = root.join(name);
            if let Ok(raw) = fs::read_to_string(&path) {
                for (idx, line) in raw.lines().enumerate() {
                    if line.to_lowercase().contains(&q) {
                        hits.push(HiveSearchHit {
                            file: name.to_string(),
                            line: idx + 1,
                            snippet: line.chars().take(200).collect(),
                        });
                        if hits.len() >= 100 {
                            return hits;
                        }
                    }
                }
            }
        }
        hits
    }

    pub fn agent_context_map(&self) -> HashMap<String, Value> {
        HashMap::new()
    }

    pub fn human_mail_since(&self, since_by_agent: &HashMap<String, i64>) -> Value {
        let mut by_agent: HashMap<String, (usize, String, Vec<String>, i64)> = HashMap::new();
        for msg in self.messages(None, 200) {
            let to = msg.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let needs_human = msg
                .get("needsHuman")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if to != "human" && !needs_human {
                continue;
            }
            let from = msg
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("god")
                .to_string();
            let ts = msg.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
            let since = since_by_agent.get(&from).copied().unwrap_or(0);
            if ts <= since {
                continue;
            }
            let subject = msg
                .get("subject")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let body = msg
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let preview = if !subject.is_empty() && !body.is_empty() {
                format!("{subject}: {body}")
            } else if !body.is_empty() {
                body.to_string()
            } else {
                subject.to_string()
            };
            let entry = by_agent
                .entry(from)
                .or_insert_with(|| (0, String::new(), Vec::new(), 0));
            entry.0 += 1;
            if ts >= entry.3 {
                entry.3 = ts;
                entry.1 = preview.clone();
            }
            if !preview.is_empty() && entry.2.len() < 10 {
                entry.2.push(preview);
            }
        }
        let mut out = Map::new();
        for (agent_id, (count, latest, texts, _)) in by_agent {
            out.insert(
                agent_id,
                serde_json::json!({
                    "count": count,
                    "latest": latest,
                    "texts": texts,
                }),
            );
        }
        Value::Object(out)
    }
}

fn ensure_tasks_array(ledger: &mut Value) -> &mut Vec<Value> {
    if !ledger.is_object() {
        *ledger = serde_json::json!({ "tasks": [] });
    }
    let obj = ledger.as_object_mut().unwrap();
    if !obj.get("tasks").is_some_and(|v| v.is_array()) {
        obj.insert("tasks".to_string(), Value::Array(Vec::new()));
    }
    obj.get_mut("tasks").unwrap().as_array_mut().unwrap()
}

fn atomic_write_json(path: &Path, val: &Value) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(val)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn read_json_dir(dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter_map(|raw| serde_json::from_str::<Value>(&raw).ok())
        .collect()
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

pub const HIVE_IGNORE_LINES: &[&str] = &[
    "fleet.json",
    "/projects/",
    "runs.json",
    "hooks.sock",
    ".floor-lock",
    "cost-ledger.jsonl",
    "log.jsonl",
    ".DS_Store",
    "crashes/",
    "node_modules/",
    ".npm-cache/",
    ".yarn/cache/",
    ".pnpm-store/",
    ".venv/",
    "__pycache__/",
    ".pytest_cache/",
    ".turbo/",
    ".next/",
    ".gradle/",
    ".remotion/",
    "/agents/*/memory.md",
    "/agents/*/memory-archive.md",
    "/agents/*/memory-archive-raw*.md",
    "/agents/*/memory-archive-laptop.md",
    "/agents/*/memory-archive-cloud.md",
    "/agents/*/**/.sync-*.tmp",
    "/sync.git/",
    "/agents/*/handoff.md",
    "/agents/*/codex-memory.md",
    "/backups/",
    "/bin/runtime/md-hook-*.cmd",
    "facts.db-shm",
    "facts.db-wal",
    "/mtime-test-*/",
    "/.palace-stage/",
    "/token-usage.json",
];

pub fn ensure_hive_gitignore(hive_root: &Path) {
    let path = hive_root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let have: HashSet<&str> = existing.lines().map(str::trim).collect();
    let missing: Vec<&str> = HIVE_IGNORE_LINES
        .iter()
        .copied()
        .filter(|line| !have.contains(line))
        .collect();
    if missing.is_empty() {
        return;
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    for line in missing {
        next.push_str(line);
        next.push('\n');
    }
    let _ = fs::write(&path, next);
}

fn real_with_tail(p: &Path) -> Option<PathBuf> {
    if !p.is_absolute() {
        return None;
    }
    let mut probe = md_core::repo_fs::normalize_lexical(p);
    let mut rest = Vec::new();
    loop {
        if let Ok(mut out) = fs::canonicalize(&probe) {
            for comp in rest.into_iter().rev() {
                out.push(comp);
            }
            return Some(out);
        }
        if fs::symlink_metadata(&probe).is_ok_and(|m| m.file_type().is_symlink()) {
            return None;
        }
        let name = probe.file_name()?.to_os_string();
        rest.push(name);
        probe = probe.parent()?.to_path_buf();
    }
}

/// Returns true iff `folder` is an absolute path strictly inside `<hive_root>/projects`
/// (with symlink traversal checks, matching `src/core/hiveProjects.ts`).
pub fn in_hive_projects(hive_root: &Path, folder: &Path) -> bool {
    let Some(base) = real_with_tail(&hive_root.join("projects")) else {
        return false;
    };
    let Some(real) = real_with_tail(folder) else {
        return false;
    };
    real != base && real.starts_with(&base)
}

/// Top-level project directory (`<hive_root>/projects/<first_component>`) for `folder`.
pub fn hive_project_top_of(hive_root: &Path, folder: &Path) -> Option<PathBuf> {
    let base = real_with_tail(&hive_root.join("projects"))?;
    let real = real_with_tail(folder)?;
    let rel = real.strip_prefix(&base).ok()?;
    let first = rel.components().next()?;
    Some(base.join(first))
}

/// Sanitizes an agent id or name into a lowercase project folder slug (`<= 64` chars).
pub fn project_folder_name(name: &str) -> String {
    let mapped: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim_matches(['-', '.']);
    let capped: String = trimmed.chars().take(64).collect();
    let clean = capped.trim_end_matches(['-', '.']);
    if clean.is_empty() {
        "project".to_string()
    } else {
        clean.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_add_patch_delete_and_search_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-hive-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut mgr = HiveManager::new(Some(dir.clone()));
        assert!(mgr
            .add_task(serde_json::json!({
                "id": "MD-1",
                "title": "Port HookServer to Axum",
                "status": "todo"
            }))
            .unwrap());
        assert!(!mgr
            .add_task(serde_json::json!({ "id": "MD-1", "title": "dup" }))
            .unwrap());

        let mut patch = Map::new();
        patch.insert("status".to_string(), Value::String("done".to_string()));
        assert!(mgr.patch_task("MD-1", &patch).unwrap());

        let hits = mgr.text_search("HookServer");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "tasks.json");

        assert!(mgr.delete_task("MD-1").unwrap());

        let active = vec![
            serde_json::json!({
                "id": "god",
                "name": "Michael",
                "isGod": true,
                "description": "Runs the office, sorts requests.",
                "provider": "opencode"
            }),
            serde_json::json!({
                "id": "eng-1",
                "name": "Engineer",
                "description": "Writes Rust",
                "provider": "claude"
            }),
        ];
        mgr.sync_roster(&active, &[]).unwrap();
        assert_eq!(mgr.registry()["agents"]["god"]["role"], "orchestrator (god)");
        assert!(mgr
            .patch_agent_field("god", "startedAt", Some(Value::Number(12345.into())))
            .unwrap());
        mgr.sync_roster(&active, &[]).unwrap();
        assert_eq!(mgr.registry()["agents"]["god"]["role"], "orchestrator (god)");
        assert_eq!(mgr.registry()["agents"]["god"]["startedAt"], 12345);
        assert!(mgr
            .patch_agent_field("eng-1", "role", Some(Value::String("Builder".to_string())))
            .unwrap());
        let reg = mgr.registry();
        assert_eq!(reg["agents"]["eng-1"]["role"], "Builder");

        // Outbox -> Inbox routing test
        let outbox_dir = dir.join("hive/agents/god/outbox");
        fs::create_dir_all(&outbox_dir).unwrap();
        fs::write(
            outbox_dir.join("order.json"),
            r#"{"to":"eng-1","act":"request","subject":"Do X","body":"Please run checks"}"#,
        )
        .unwrap();
        let routed = mgr.drain_outboxes();
        assert_eq!(routed.len(), 1);
        assert_eq!(mgr.inbox("eng-1").len(), 1);
        assert!(!outbox_dir.join("order.json").exists());
        assert!(outbox_dir.join(".sent/order.json").exists());

        // god -> human must NOT loop back into god/inbox; worker -> human routes to god
        fs::write(
            outbox_dir.join("ask-human.json"),
            r#"{"to":"human","act":"ask","subject":"Status?","body":"What next?"}"#,
        )
        .unwrap();
        let worker_outbox = dir.join("hive/agents/eng-1/outbox");
        fs::create_dir_all(&worker_outbox).unwrap();
        fs::write(
            worker_outbox.join("worker-human.json"),
            r#"{"to":"human","act":"inform","subject":"Review","body":"Ready"}"#,
        )
        .unwrap();
        let routed_human = mgr.drain_outboxes();
        assert_eq!(routed_human.len(), 2);
        let god_inbox = mgr.inbox("god");
        assert_eq!(god_inbox.len(), 1);
        assert_eq!(god_inbox[0]["from"], "eng-1");
        assert!(outbox_dir.join(".sent/ask-human.json").exists());

        // Removing a non-temp agent from roster archives it in registry.json
        mgr.sync_roster(&[], &[]).unwrap();
        let reg2 = mgr.registry();
        assert_eq!(reg2["agents"]["eng-1"]["archived"], true);

        let gi = fs::read_to_string(dir.join("hive/.gitignore")).unwrap();
        assert!(gi.contains("/projects/"));
        assert!(gi.contains("fleet.json"));
        assert!(in_hive_projects(&dir.join("hive"), &dir.join("hive/projects/anvil-gtm")));
        assert!(!in_hive_projects(&dir.join("hive"), &dir.join("hive/projects")));
        assert!(!in_hive_projects(&dir.join("hive"), &dir.join("hive/agents")));
        assert_eq!(project_folder_name("Worker ACME 1!"), "worker-acme-1");

        // Unkeyed tasks written directly to tasks.json are auto-keyed from home initials.
        let tasks_path = dir.join("hive/tasks.json");
        fs::write(
            &tasks_path,
            serde_json::json!({
                "tasks": [
                    { "title": "First card", "status": "todo" },
                    { "id": "CUSTOM-9", "title": "Pre-keyed", "status": "doing" },
                    { "title": "Third card", "status": "blocked" }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let keyed = mgr.tasks();
        let arr = keyed["tasks"].as_array().unwrap();
        assert_eq!(arr[0]["id"].as_str().unwrap(), &format!("{}-1", mgr.default_ticket_prefix(&dir.join("hive"))));
        assert_eq!(arr[1]["id"], "CUSTOM-9");
        assert_eq!(arr[2]["id"].as_str().unwrap(), &format!("{}-2", mgr.default_ticket_prefix(&dir.join("hive"))));

        mgr.append_log_with_cap(&serde_json::json!({ "kind": "rot-1", "msg": "aaaa" }), 20);
        mgr.append_log_with_cap(&serde_json::json!({ "kind": "rot-2", "msg": "bbbb" }), 20);
        assert!(dir.join("hive/log.jsonl.1").exists());
        assert!(dir.join("hive/log.jsonl").exists());

        // Regression: lagged UI roster sync must not clobber hook-driven status.
        mgr.patch_agent_field("god", "status", Some(Value::String("idle".to_string())))
            .unwrap();
        let stale_roster = vec![serde_json::json!({
            "id": "god",
            "name": "Michael",
            "isGod": true,
            "provider": "opencode",
            "status": "working"
        })];
        mgr.sync_roster(&stale_roster, &[]).unwrap();
        assert_eq!(mgr.registry()["agents"]["god"]["status"], "idle");

        fs::remove_dir_all(&dir).ok();
    }
}
