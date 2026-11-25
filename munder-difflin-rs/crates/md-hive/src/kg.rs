//! File-backed Enterprise Knowledge Graph compatible with `kg-core.cjs`
//! layout (`index.jsonl` + `docs/<docId>/{meta.json, text.md, original.<ext>}`).

use md_core::repo_fs::is_safe_id;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;

const DEFAULT_CHUNK_SIZE: usize = 1200;
const DEFAULT_CHUNK_OVERLAP: usize = 150;
const MAX_INGEST_BYTES: u64 = 50 * 1024 * 1024;
const MAX_INDEX_BYTES: usize = 5 * 1024 * 1024;
const SNIPPET_RADIUS: usize = 160;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "how",
    "in", "is", "it", "its", "of", "on", "or", "that", "the", "their", "this", "to", "was", "were",
    "what", "when", "where", "which", "who", "will", "with", "your", "you", "we", "our", "us",
    "they", "them", "he", "she", "his", "her", "me", "my", "do", "does", "did", "not", "can",
    "could", "would", "should", "about", "into", "over", "than", "then", "there", "here", "so",
    "if", "no", "yes",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgMeta {
    pub id: String,
    pub title: String,
    pub source: String,
    pub modality: String,
    pub mime: Option<String>,
    pub orig_ext: String,
    pub bytes: u64,
    pub tags: Vec<String>,
    pub caption: Option<String>,
    pub chunk_count: usize,
    pub added_at: String,
    pub extractor: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgHit {
    pub doc_id: String,
    pub title: String,
    pub source: String,
    pub modality: String,
    pub chunk_idx: usize,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KgStatus {
    pub enabled: bool,
    pub root: String,
    pub doc_count: usize,
    pub chunk_count: usize,
    pub by_modality: HashMap<String, usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KgIngestInput {
    pub id: Option<String>,
    pub src_path: Option<String>,
    pub text: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub caption: Option<String>,
    pub modality: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChunkRecord {
    doc_id: String,
    title: String,
    source: String,
    modality: String,
    chunk_idx: usize,
    text: String,
}

fn tokenize(s: &str) -> Vec<String> {
    let stop: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            if cur.len() > 1 && !stop.contains(cur.as_str()) {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.len() > 1 && !stop.contains(cur.as_str()) {
        out.push(cur);
    }
    out
}

fn derive_title(text: &str) -> String {
    for raw in text.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        let heading = t.trim_start_matches('#').trim();
        let chars: Vec<char> = heading.chars().collect();
        if chars.len() > 80 {
            let short: String = chars[..80].iter().collect();
            return format!("{short}…");
        }
        return heading.to_string();
    }
    String::new()
}

fn chunk_text(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let t = normalized.trim();
    if t.is_empty() {
        return Vec::new();
    }
    if t.len() <= DEFAULT_CHUNK_SIZE {
        return vec![t.to_string()];
    }
    let mut chunks = Vec::new();
    let mut i = 0usize;
    while i < t.len() {
        let mut end = (i + DEFAULT_CHUNK_SIZE).min(t.len());
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        if end < t.len() {
            let win = &t[i..end];
            let br = win
                .rfind("\n\n")
                .or_else(|| win.rfind('\n'))
                .or_else(|| win.rfind(' '));
            if let Some(pos) = br
                && pos > (DEFAULT_CHUNK_SIZE * 6) / 10
            {
                end = i + pos;
            }
        }
        let piece = t[i..end].trim();
        if !piece.is_empty() {
            chunks.push(piece.to_string());
        }
        if end >= t.len() {
            break;
        }
        let mut next_i = end.saturating_sub(DEFAULT_CHUNK_OVERLAP).max(i + 1);
        while next_i < t.len() && !t.is_char_boundary(next_i) {
            next_i += 1;
        }
        i = next_i;
    }
    chunks
}

fn score_chunk(rec: &ChunkRecord, query_terms: &[String], query_raw: &str) -> f64 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let mut tf: HashMap<String, usize> = HashMap::new();
    for tok in tokenize(&rec.text) {
        *tf.entry(tok).or_insert(0) += 1;
    }
    let title_lc = rec.title.to_ascii_lowercase();
    let mut score = 0.0;
    let mut matched = 0usize;
    for term in query_terms {
        let c = *tf.get(term).unwrap_or(&0);
        if c > 0 {
            matched += 1;
            score += 1.0 + (1.0 + c as f64).ln();
        }
        if title_lc.contains(term) {
            score += 2.0;
        }
    }
    if matched == 0 {
        return 0.0;
    }
    score += matched as f64 * 0.5;
    let q = query_raw.trim().to_ascii_lowercase();
    if q.len() >= 3 && rec.text.to_ascii_lowercase().contains(&q) {
        score += 5.0;
    }
    score
}

fn make_snippet(text: &str, query_terms: &[String]) -> String {
    let compact: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return String::new();
    }
    let lc = compact.to_ascii_lowercase();
    let hit = query_terms
        .iter()
        .filter_map(|t| lc.find(t))
        .min();
    let Some(at) = hit else {
        return compact.chars().take(SNIPPET_RADIUS * 2).collect();
    };
    let mut start = at.saturating_sub(SNIPPET_RADIUS);
    while start > 0 && !compact.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + SNIPPET_RADIUS).min(compact.len());
    while end < compact.len() && !compact.is_char_boundary(end) {
        end += 1;
    }
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        compact[start..end].trim(),
        if end < compact.len() { "…" } else { "" }
    )
}

fn detect_modality(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" => "image",
        "pdf" => "pdf",
        "csv" | "tsv" => "sheet",
        "ts" | "tsx" | "js" | "jsx" | "rs" | "py" | "go" | "java" | "c" | "cpp" | "sh" | "sql" => {
            "code"
        }
        _ => "text",
    }
}

pub fn ingest(kg_root: &Path, input: KgIngestInput) -> Result<KgMeta, String> {
    let doc_id = match input.id {
        Some(ref id) => {
            if !is_safe_id(id) {
                return Err("invalid document id".to_string());
            }
            id.clone()
        }
        None => uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
    };

    let mut src_bytes = 0u64;
    let mut orig_ext = "txt".to_string();
    let mut file_text = String::new();
    let mut extractor = "inline@1";

    if let Some(ref src) = input.src_path {
        let p = Path::new(src);
        let meta = fs::metadata(p).map_err(|e| format!("source unreadable: {e}"))?;
        if !meta.is_file() {
            return Err("source path is not a regular file".to_string());
        }
        if meta.len() > MAX_INGEST_BYTES {
            return Err("source exceeds 50 MiB limit".to_string());
        }
        src_bytes = meta.len();
        orig_ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        file_text = fs::read_to_string(p).unwrap_or_default();
        extractor = "text-utf8@1";
    }

    let docs_dir = kg_root.join("docs");
    let doc_dir = docs_dir.join(&doc_id);
    fs::create_dir_all(&doc_dir).map_err(|e| e.to_string())?;

    if let Some(ref src) = input.src_path {
        let _ = fs::copy(src, doc_dir.join(format!("original.{orig_ext}")));
    }

    let source = input.source.clone().unwrap_or_else(|| {
        input
            .src_path
            .as_deref()
            .and_then(|s| Path::new(s).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| input.title.clone())
            .unwrap_or_else(|| "untitled".to_string())
    });
    let modality = input.modality.clone().unwrap_or_else(|| {
        input
            .src_path
            .as_deref()
            .map(|s| detect_modality(Path::new(s)).to_string())
            .unwrap_or_else(|| "text".to_string())
    });

    let full_text = input.text.clone().unwrap_or(file_text);
    fs::write(doc_dir.join("text.md"), &full_text).map_err(|e| e.to_string())?;

    let mut truncated = false;
    let index_text = if full_text.len() > MAX_INDEX_BYTES {
        truncated = true;
        let mut end = MAX_INDEX_BYTES;
        while !full_text.is_char_boundary(end) {
            end -= 1;
        }
        &full_text[..end]
    } else {
        &full_text
    };

    let derived = derive_title(&full_text);
    let title = input
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or(if derived.is_empty() {
            None
        } else {
            Some(derived)
        })
        .unwrap_or_else(|| source.clone());

    let mut chunks = chunk_text(index_text);
    if chunks.is_empty() {
        chunks.push(title.clone());
    }

    let idx_path = kg_root.join("index.jsonl");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&idx_path)
        .map_err(|e| e.to_string())?;
    for (chunk_idx, text) in chunks.iter().enumerate() {
        let rec = ChunkRecord {
            doc_id: doc_id.clone(),
            title: title.clone(),
            source: source.clone(),
            modality: modality.clone(),
            chunk_idx,
            text: text.clone(),
        };
        let line = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
    }

    let meta = KgMeta {
        id: doc_id,
        title,
        source,
        modality,
        mime: Some("text/plain".to_string()),
        orig_ext,
        bytes: src_bytes,
        tags: input.tags,
        caption: input.caption,
        chunk_count: chunks.len(),
        added_at: chrono::Utc::now().to_rfc3339(),
        extractor: extractor.to_string(),
        truncated,
    };
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(doc_dir.join("meta.json"), meta_json).map_err(|e| e.to_string())?;
    Ok(meta)
}

pub fn list(kg_root: &Path) -> Vec<KgMeta> {
    let docs_dir = kg_root.join("docs");
    let Ok(rd) = fs::read_dir(&docs_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path().join("meta.json");
        if let Ok(raw) = fs::read_to_string(p)
            && let Ok(meta) = serde_json::from_str::<KgMeta>(&raw)
        {
            out.push(meta);
        }
    }
    out.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    out
}

pub fn get_doc(kg_root: &Path, doc_id: &str) -> Option<(KgMeta, String)> {
    if !is_safe_id(doc_id) {
        return None;
    }
    let doc_dir = kg_root.join("docs").join(doc_id);
    let meta: KgMeta = serde_json::from_str(&fs::read_to_string(doc_dir.join("meta.json")).ok()?)
        .ok()?;
    let text = fs::read_to_string(doc_dir.join("text.md")).unwrap_or_default();
    Some((meta, text))
}

pub fn remove_doc(kg_root: &Path, doc_id: &str) -> bool {
    if !is_safe_id(doc_id) {
        return false;
    }
    let doc_dir = kg_root.join("docs").join(doc_id);
    if fs::symlink_metadata(&doc_dir)
        .map(|m| !m.is_dir())
        .unwrap_or(true)
    {
        return false;
    }
    if fs::remove_dir_all(&doc_dir).is_err() {
        return false;
    }
    let idx_path = kg_root.join("index.jsonl");
    if let Ok(raw) = fs::read_to_string(&idx_path) {
        let kept: Vec<&str> = raw
            .lines()
            .filter(|line| {
                serde_json::from_str::<ChunkRecord>(line)
                    .map(|r| r.doc_id != doc_id)
                    .unwrap_or(false)
            })
            .collect();
        let next = if kept.is_empty() {
            String::new()
        } else {
            format!("{}\n", kept.join("\n"))
        };
        let _ = fs::write(idx_path, next);
    }
    true
}

pub fn search(kg_root: &Path, query: &str, limit: usize) -> Vec<KgHit> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Vec::new();
    }
    let idx_path = kg_root.join("index.jsonl");
    let Ok(raw) = fs::read_to_string(&idx_path) else {
        return Vec::new();
    };
    let mut scored = Vec::new();
    for line in raw.lines() {
        let Ok(rec) = serde_json::from_str::<ChunkRecord>(line) else {
            continue;
        };
        let sc = score_chunk(&rec, &query_terms, query);
        if sc > 0.0 {
            scored.push((rec, sc));
        }
    }
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.doc_id.cmp(&b.0.doc_id))
            .then_with(|| a.0.chunk_idx.cmp(&b.0.chunk_idx))
    });
    scored
        .into_iter()
        .take(limit.clamp(1, 100))
        .map(|(rec, sc)| KgHit {
            snippet: make_snippet(&rec.text, &query_terms),
            doc_id: rec.doc_id,
            title: rec.title,
            source: rec.source,
            modality: rec.modality,
            chunk_idx: rec.chunk_idx,
            score: (sc * 1000.0).round() / 1000.0,
        })
        .collect()
}

pub fn status(kg_root: &Path, enabled: bool) -> KgStatus {
    let docs = if enabled && kg_root.exists() {
        list(kg_root)
    } else {
        Vec::new()
    };
    let mut chunk_count = 0usize;
    let mut by_modality = HashMap::new();
    for d in &docs {
        chunk_count += d.chunk_count;
        *by_modality.entry(d.modality.clone()).or_insert(0) += 1;
    }
    KgStatus {
        enabled,
        root: kg_root.to_string_lossy().into_owned(),
        doc_count: docs.len(),
        chunk_count,
        by_modality,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kg_ingest_search_get_and_remove_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-kg-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let meta = ingest(
            &dir,
            KgIngestInput {
                title: Some("Runbook".into()),
                text: Some("# Architecture\nMunder Difflin uses pure Rust Axum.\n".into()),
                tags: vec!["rust".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let st = status(&dir, true);
        assert_eq!(st.doc_count, 1);

        let hits = search(&dir, "Rust Axum", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, meta.id);

        assert!(get_doc(&dir, &meta.id).is_some());
        assert!(remove_doc(&dir, &meta.id));
        assert_eq!(status(&dir, true).doc_count, 0);
        fs::remove_dir_all(&dir).ok();
    }
}
