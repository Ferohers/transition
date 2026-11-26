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
        let board = root.join("board.md");
        if !board.exists() {
            fs::write(
                board,
                "# Hive board\n\n_Shared plans live here. The god agent is the scribe._\n",
            )?;
        }
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
        for (list, is_archived) in [(agents, false), (archived, true)] {
            for item in list {
                let Some(obj) = item.as_object() else { continue };
                let Some(id) = obj.get("id").and_then(|v| v.as_str()).filter(|s| is_safe_id(s)) else {
                    continue;
                };
                if let Some(existing) = agents_map.get_mut(id).and_then(|v| v.as_object_mut()) {
                    for k in ["name", "provider", "cwd", "isGod"] {
                        if let Some(v) = obj.get(k).cloned()
                            && existing.get(k) != Some(&v)
                        {
                            existing.insert(k.to_string(), v);
                            changed = true;
                        }
                    }
                    continue;
                }
                let mut entry = Map::new();
                entry.insert("id".to_string(), Value::String(id.to_string()));
                if let Some(name) = obj.get("name").cloned() {
                    entry.insert("name".to_string(), name);
                }
                if let Some(role) = obj.get("role").or_else(|| obj.get("description")).cloned() {
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
            ReadOutcome::Ok(v) => v,
            _ => serde_json::json!({ "tasks": [] }),
        }
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
        let Ok(root) = self.ensure_hive() else {
            return;
        };
        let path = root.join("log.jsonl");
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
                    vec!["god".to_string()]
                } else if is_safe_id(&to_raw) {
                    vec![to_raw.clone()]
                } else {
                    Vec::new()
                };

                if targets.is_empty() && to_raw != "broadcast" {
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

        let active = vec![serde_json::json!({
            "id": "eng-1",
            "name": "Engineer",
            "description": "Writes Rust",
            "provider": "claude"
        })];
        mgr.sync_roster(&active, &[]).unwrap();
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

        fs::remove_dir_all(&dir).ok();
    }
}
