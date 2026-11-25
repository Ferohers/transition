//! Atomic roster mirror (`roster.json`) with `roster-backups/` pruning and
//! empty-first-write protection, mirroring `RosterStore` from Munder Difflin 0.5.10.

use chrono::Utc;
use md_core::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const PRUNE_EVERY: usize = 25;
const KEEP_NEWEST_BACKUPS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RosterSnapshot {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub saved_at: Option<String>,
    pub agents: Vec<Value>,
    pub archived: Vec<Value>,
    pub restorable: Vec<Value>,
    #[serde(default)]
    pub queues: HashMap<String, Value>,
    #[serde(default)]
    pub selected_id: Option<String>,
}

fn default_version() -> u32 {
    1
}

impl RosterSnapshot {
    pub fn entry_count(&self) -> usize {
        self.agents.len() + self.archived.len() + self.restorable.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RosterWriteResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct RosterStore {
    wrote: bool,
    backup_seq: u64,
    since_reap: usize,
}

impl Default for RosterStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RosterStore {
    pub fn new() -> Self {
        Self {
            wrote: false,
            backup_seq: 0,
            since_reap: 0,
        }
    }

    pub fn roster_path(home: &Path) -> PathBuf {
        home.join("roster.json")
    }

    pub fn backup_dir(home: &Path) -> PathBuf {
        home.join("roster-backups")
    }

    pub fn read(&self, home: Option<&Path>) -> Option<RosterSnapshot> {
        let path = Self::roster_path(home?);
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<RosterSnapshot>(&raw).ok()
    }

    pub fn write(&mut self, home: Option<&Path>, snap: &RosterSnapshot) -> RosterWriteResult {
        let Some(home) = home else {
            return RosterWriteResult {
                ok: false,
                skipped: None,
                error: Some("no harnessHome".to_string()),
            };
        };

        let path = Self::roster_path(home);
        if let Err(e) = fs::create_dir_all(home) {
            return RosterWriteResult {
                ok: false,
                skipped: None,
                error: Some(e.to_string()),
            };
        }

        let existing = self.read(Some(home));
        if !self.wrote
            && existing.as_ref().is_some_and(|e| e.entry_count() > 0)
            && snap.entry_count() == 0
        {
            self.backup(home, &path, "declined");
            return RosterWriteResult {
                ok: false,
                skipped: Some("empty-first-write".to_string()),
                error: None,
            };
        }

        let reason = if self.wrote { "write" } else { "run-start" };
        self.backup(home, &path, reason);

        let body = RosterSnapshot {
            version: 1,
            saved_at: Some(Utc::now().to_rfc3339()),
            agents: snap.agents.clone(),
            archived: snap.archived.clone(),
            restorable: snap.restorable.clone(),
            queues: snap.queues.clone(),
            selected_id: snap.selected_id.clone(),
        };

        let tmp = path.with_extension("json.tmp");
        let write_res: Result<(), CoreError> = (|| {
            let text = serde_json::to_string_pretty(&body)?;
            fs::write(&tmp, text)?;
            fs::rename(&tmp, &path)?;
            Ok(())
        })();

        match write_res {
            Ok(()) => {
                self.wrote = true;
                RosterWriteResult {
                    ok: true,
                    skipped: None,
                    error: None,
                }
            }
            Err(e) => {
                fs::remove_file(&tmp).ok();
                RosterWriteResult {
                    ok: false,
                    skipped: None,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    fn backup(&mut self, home: &Path, path: &Path, reason: &str) {
        if !path.exists() {
            return;
        }
        let dir = Self::backup_dir(home);
        if fs::create_dir_all(&dir).is_ok() {
            self.backup_seq += 1;
            let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%S-%3fZ");
            let dest = dir.join(format!("roster-{stamp}-{}-{reason}.json", self.backup_seq));
            fs::copy(path, dest).ok();
        }
        self.since_reap += 1;
        if self.since_reap >= PRUNE_EVERY {
            self.since_reap = 0;
            reap_backups(&dir);
        }
    }
}

fn reap_backups(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    if files.len() <= KEEP_NEWEST_BACKUPS {
        return;
    }
    files.sort_by_key(|e| e.file_name());
    let drop_count = files.len() - KEEP_NEWEST_BACKUPS;
    for entry in files.into_iter().take(drop_count) {
        fs::remove_file(entry.path()).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_first_write_guard_protects_existing_roster() {
        let dir = std::env::temp_dir().join(format!(
            "md-roster-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let mut writer = RosterStore::new();
        let initial = RosterSnapshot {
            version: 1,
            saved_at: None,
            agents: vec![serde_json::json!({ "id": "michael", "name": "Michael" })],
            archived: vec![],
            restorable: vec![],
            queues: HashMap::new(),
            selected_id: Some("michael".to_string()),
        };
        assert!(writer.write(Some(&dir), &initial).ok);

        // A fresh boot store refuses to clobber a non-empty roster with an empty snapshot.
        let mut fresh_boot = RosterStore::new();
        let empty = RosterSnapshot {
            version: 1,
            saved_at: None,
            agents: vec![],
            archived: vec![],
            restorable: vec![],
            queues: HashMap::new(),
            selected_id: None,
        };
        let blocked = fresh_boot.write(Some(&dir), &empty);
        assert!(!blocked.ok);
        assert_eq!(blocked.skipped.as_deref(), Some("empty-first-write"));
        assert_eq!(fresh_boot.read(Some(&dir)).unwrap().agents.len(), 1);

        fs::remove_dir_all(&dir).ok();
    }
}
