//! Local agent memory management (`agents/<id>/memory.md`), atomic
//! `memory-index.md` generation, and local keyword memory search.

use md_core::repo_fs::is_safe_id;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MEMORY_INDEX_FILE: &str = "memory-index.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    pub available: bool,
    pub enabled: bool,
    pub active: bool,
    pub initialized: bool,
    pub palace_path: Option<String>,
    pub model: String,
    pub agent_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIndexRow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub path: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub empty: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchHit {
    pub agent_id: String,
    pub line: usize,
    pub snippet: String,
    pub path: String,
}

fn agent_memory_path(home: &Path, agent_id: &str) -> Result<PathBuf, String> {
    if !is_safe_id(agent_id) {
        return Err("invalid agent id".to_string());
    }
    Ok(home.join("hive").join("agents").join(agent_id).join("memory.md"))
}

pub fn read_agent_memory(home: &Path, agent_id: &str) -> Result<String, String> {
    let path = agent_memory_path(home, agent_id)?;
    if !path.is_file() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

pub fn write_agent_memory(home: &Path, agent_id: &str, content: &str) -> Result<String, String> {
    let path = agent_memory_path(home, agent_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn list_memory_rows(home: &Path) -> Vec<MemoryIndexRow> {
    let agents_dir = home.join("hive").join("agents");
    let Ok(rd) = fs::read_dir(&agents_dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in rd.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if !is_safe_id(&id) {
            continue;
        }
        let mem_path = dir.join("memory.md");
        let (bytes, empty, updated_at) = match fs::metadata(&mem_path) {
            Ok(meta) => {
                let b = meta.len();
                let body = fs::read_to_string(&mem_path).unwrap_or_default();
                let non_header = body
                    .lines()
                    .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
                let ts = meta.modified().ok().map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                });
                (b, !non_header, ts)
            }
            Err(_) => (0, true, None),
        };
        rows.push(MemoryIndexRow {
            name: id.clone(),
            id,
            role: None,
            path: mem_path.to_string_lossy().into_owned(),
            bytes,
            updated_at,
            empty,
            archived: false,
        });
    }
    rows.sort_by(|a, b| a.empty.cmp(&b.empty).then_with(|| a.id.cmp(&b.id)));
    rows
}

pub fn write_memory_index(home: &Path) -> Result<String, String> {
    let root = home.join("hive");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let rows = list_memory_rows(home);
    let with_notes = rows.iter().filter(|r| !r.empty).count();
    let mut lines = vec![
        "# Memory index".to_string(),
        String::new(),
        format!(
            "{} agent{} registered on this floor, {} with notes.",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" },
            with_notes
        ),
        String::new(),
    ];
    for r in &rows {
        let state = if r.empty { "empty" } else { "active" };
        lines.push(format!("{}  {}  {}  {} B", r.id, r.name, state, r.bytes));
        lines.push(format!("  {}", r.path));
        lines.push(String::new());
    }
    let idx_path = root.join(MEMORY_INDEX_FILE);
    let tmp = root.join(format!("{MEMORY_INDEX_FILE}.tmp"));
    fs::write(&tmp, lines.join("\n")).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &idx_path).map_err(|e| e.to_string())?;
    Ok(idx_path.to_string_lossy().into_owned())
}

pub fn search_memory(home: &Path, query: &str, limit: usize) -> Vec<MemorySearchHit> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let cap = limit.clamp(1, 200);
    let mut hits = Vec::new();
    for row in list_memory_rows(home) {
        let Ok(body) = fs::read_to_string(&row.path) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&q) {
                hits.push(MemorySearchHit {
                    agent_id: row.id.clone(),
                    line: idx + 1,
                    snippet: line.chars().take(240).collect(),
                    path: row.path.clone(),
                });
                if hits.len() >= cap {
                    return hits;
                }
            }
        }
    }
    hits
}

pub fn memory_status(home: Option<&Path>, enabled: bool, model: Option<&str>) -> MemoryStatus {
    let palace_path = home.map(|h| h.join("palace").to_string_lossy().into_owned());
    let initialized = home.map(|h| h.join("palace").is_dir()).unwrap_or(false);
    let agent_files = home
        .map(|h| list_memory_rows(h).into_iter().filter(|r| !r.empty).count())
        .unwrap_or(0);
    MemoryStatus {
        available: true,
        enabled,
        active: enabled && home.is_some(),
        initialized,
        palace_path,
        model: model.unwrap_or("local-keyword").to_string(),
        agent_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_indexes_and_searches_agent_memory() {
        let dir = std::env::temp_dir().join(format!(
            "md-mem-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_agent_memory(&dir, "michael", "# Notes\nPro view uses compact density.\n").unwrap();
        assert!(read_agent_memory(&dir, "michael").unwrap().contains("compact density"));
        let idx = write_memory_index(&dir).unwrap();
        assert!(Path::new(&idx).is_file());
        let hits = search_memory(&dir, "compact", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].agent_id, "michael");
        fs::remove_dir_all(&dir).ok();
    }
}
