//! Safe JSON file reader & damage ledger mirroring `DamagedFiles` from `out/main/index.js`.

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
}

impl FileIdentity {
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            ino: meta.ino(),
            size: meta.size(),
            mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
            ctime_ns: meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonShape {
    Object,
    Registry,
    Tasks,
}

pub fn shape_problem(value: &serde_json::Value, shape: JsonShape) -> Option<String> {
    let Some(obj) = value.as_object() else {
        return Some("the file does not hold an object".to_string());
    };
    match shape {
        JsonShape::Object => None,
        JsonShape::Tasks => {
            if let Some(tasks) = obj.get("tasks")
                && !tasks.is_array()
            {
                return Some("\"tasks\" is not a list".to_string());
            }
            None
        }
        JsonShape::Registry => match obj.get("agents") {
            Some(v) if v.is_object() => None,
            _ => Some("\"agents\" is not an object".to_string()),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityIssue {
    pub file: String,
    pub copy: Option<String>,
    pub error: String,
}

#[derive(Debug, Clone)]
struct RecordedIssue {
    file: String,
    copy: Option<String>,
    error: String,
    id: Option<FileIdentity>,
}

#[derive(Debug)]
pub enum ReadOutcome {
    Ok(serde_json::Value),
    Missing,
    Damaged,
    Unreadable,
}

#[derive(Debug, Default)]
pub struct DamagedFiles {
    issues: HashMap<PathBuf, RecordedIssue>,
    copied: HashSet<PathBuf>,
    good: HashMap<PathBuf, FileIdentity>,
}

impl DamagedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self, path: &Path) {
        self.issues.remove(path);
        self.copied.remove(path);
    }

    pub fn read(&mut self, path: &Path, shape: JsonShape) -> ReadOutcome {
        if !path.exists() {
            self.clear(path);
            return ReadOutcome::Missing;
        }
        let id = FileIdentity::of(path);
        if let (Some(known), Some(cur_id)) = (self.issues.get(path), id.as_ref())
            && known.id.as_ref() == Some(cur_id)
        {
            return ReadOutcome::Damaged;
        }
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                self.note(path, None, None, e.to_string());
                return ReadOutcome::Unreadable;
            }
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                self.note(path, id, Some(&raw), e.to_string());
                return ReadOutcome::Damaged;
            }
        };
        if let Some(problem) = shape_problem(&parsed, shape) {
            self.note(path, id, Some(&raw), problem);
            return ReadOutcome::Damaged;
        }
        self.clear(path);
        if let Some(cur_id) = id {
            self.good.insert(path.to_path_buf(), cur_id);
        } else {
            self.good.remove(path);
        }
        ReadOutcome::Ok(parsed)
    }

    pub fn is_damaged(&mut self, path: &Path, shape: JsonShape) -> bool {
        if let Some(id) = FileIdentity::of(path)
            && self.good.get(path) == Some(&id)
            && !self.issues.contains_key(path)
        {
            return false;
        }
        !matches!(self.read(path, shape), ReadOutcome::Ok(_) | ReadOutcome::Missing)
    }

    pub fn mark_wrote(&mut self, path: &Path) {
        let id = FileIdentity::of(path);
        self.clear(path);
        if let Some(cur_id) = id {
            self.good.insert(path.to_path_buf(), cur_id);
        } else {
            self.good.remove(path);
        }
    }

    pub fn list(&self) -> Vec<IntegrityIssue> {
        self.issues
            .values()
            .map(|i| IntegrityIssue {
                file: i.file.clone(),
                copy: i.copy.clone(),
                error: i.error.clone(),
            })
            .collect()
    }

    fn note(&mut self, path: &Path, id: Option<FileIdentity>, raw: Option<&str>, error: String) {
        let prev_copy = self.issues.get(path).and_then(|i| i.copy.clone());
        let mut copy = prev_copy;
        if let Some(content) = raw
            && !self.copied.contains(path)
        {
            self.copied.insert(path.to_path_buf());
            let dest = damaged_copy_path(path);
            if fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(content.as_bytes())
                })
                .is_ok()
            {
                copy = dest
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned());
            }
        }
        let file = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.issues.insert(
            path.to_path_buf(),
            RecordedIssue {
                file,
                copy,
                error,
                id,
            },
        );
    }
}

fn damaged_copy_path(path: &Path) -> PathBuf {
    let now = Local::now();
    let base = format!(
        "{}.damaged-{}",
        path.display(),
        now.format("%Y-%m-%d-%H%M")
    );
    let candidate = PathBuf::from(&base);
    if !candidate.exists() {
        return candidate;
    }
    let with_sec = PathBuf::from(format!("{base}{}", now.format("%S")));
    if !with_sec.exists() {
        return with_sec;
    }
    for n in 0u32.. {
        let name = if n == 0 {
            format!("{}-{}", with_sec.display(), now.format("%3f"))
        } else {
            format!("{}-{}-{n}", with_sec.display(), now.format("%3f"))
        };
        let p = PathBuf::from(name);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_json_is_quarantined_and_copied_once() {
        let dir = std::env::temp_dir().join(format!("md-dmg-{}", uuid_stub()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");
        fs::write(&file, "{broken json").unwrap();

        let mut ledger = DamagedFiles::new();
        assert!(ledger.is_damaged(&file, JsonShape::Object));
        let issues = ledger.list();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].file, "config.json");
        assert!(issues[0].copy.is_some());

        // Repairing the file clears the issue on next read.
        fs::write(&file, r#"{"ok":true}"#).unwrap();
        assert!(!ledger.is_damaged(&file, JsonShape::Object));
        assert!(ledger.list().is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    fn uuid_stub() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
