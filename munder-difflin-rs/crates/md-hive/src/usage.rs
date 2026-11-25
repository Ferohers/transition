//! Local Claude/Codex transcript parser, per-model pricing estimator, and
//! tail context-window reader (replaces OTLP collector paths cleanly).

use md_core::repo_fs::{is_safe_id, open_nofollow_read};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CONTEXT_TAIL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input_per_m: f64,
    pub output_per_m: f64,
    pub cache_read_per_m: f64,
    pub cache_write_per_m: f64,
}

const OPUS_PRICE: ModelPrice = ModelPrice {
    input_per_m: 15.0,
    output_per_m: 75.0,
    cache_read_per_m: 1.5,
    cache_write_per_m: 18.75,
};
const SONNET_PRICE: ModelPrice = ModelPrice {
    input_per_m: 3.0,
    output_per_m: 15.0,
    cache_read_per_m: 0.3,
    cache_write_per_m: 3.75,
};
const HAIKU_PRICE: ModelPrice = ModelPrice {
    input_per_m: 0.8,
    output_per_m: 4.0,
    cache_read_per_m: 0.08,
    cache_write_per_m: 1.0,
};
const SONNET_5_5_PRICE: ModelPrice = ModelPrice {
    input_per_m: 2.0,
    output_per_m: 10.0,
    cache_read_per_m: 0.2,
    cache_write_per_m: 2.5,
};
const FABLE_5_2_PRICE: ModelPrice = ModelPrice {
    input_per_m: 8.0,
    output_per_m: 40.0,
    cache_read_per_m: 0.2,
    cache_write_per_m: 10.0,
};
const GPT_6_1_SOL_PRICE: ModelPrice = ModelPrice {
    input_per_m: 2.0,
    output_per_m: 10.0,
    cache_read_per_m: 0.1,
    cache_write_per_m: 2.0,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub estimated_cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub as_of: u64,
}

/// Strip bracketed variant suffixes like `claude-opus-4-8[1m]`.
pub fn normalize_model(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.ends_with(']')
        && let Some(idx) = trimmed.rfind('[')
    {
        return trimmed[..idx].trim_end().to_string();
    }
    trimmed.to_string()
}

pub fn price_for(model: Option<&str>) -> ModelPrice {
    let m = normalize_model(model.unwrap_or("")).to_ascii_lowercase();
    if m.contains("opus") {
        OPUS_PRICE
    } else if m.contains("haiku") {
        HAIKU_PRICE
    } else if m.contains("sonnet-5-5") {
        SONNET_5_5_PRICE
    } else if m.contains("fable-5-2") {
        FABLE_5_2_PRICE
    } else if m.contains("gpt-6.1-sol") {
        GPT_6_1_SOL_PRICE
    } else {
        SONNET_PRICE
    }
}

pub fn estimate_cost_usd(
    model: Option<&str>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    let p = price_for(model);
    (input_tokens as f64 / 1_000_000.0) * p.input_per_m
        + (output_tokens as f64 / 1_000_000.0) * p.output_per_m
        + (cache_read_tokens as f64 / 1_000_000.0) * p.cache_read_per_m
        + (cache_write_tokens as f64 / 1_000_000.0) * p.cache_write_per_m
}

/// Claude Code's project key: every non-ASCII-alphanumeric character becomes `-`.
pub fn project_key(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn legacy_project_key(cwd: &str) -> String {
    cwd.trim_start_matches('/').replace('/', "-")
}

pub fn default_projects_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("projects")
}

pub fn project_dir_in(projects_root: &Path, cwd: &str) -> PathBuf {
    let current = projects_root.join(project_key(cwd));
    if current.exists() {
        return current;
    }
    let legacy = legacy_project_key(cwd);
    if legacy.is_empty() {
        return current;
    }
    let legacy_dir = projects_root.join(legacy);
    if legacy_dir.exists() {
        legacy_dir
    } else {
        current
    }
}

fn as_u64(v: Option<&Value>) -> u64 {
    v.and_then(|val| {
        val.as_u64()
            .or_else(|| val.as_f64().map(|f| f.max(0.0) as u64))
    })
    .unwrap_or(0)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn accumulate_line(line: &str, all: &mut AgentUsage, per_session: &mut HashMap<String, AgentUsage>) {
    if !line.contains("\"usage\"") {
        return;
    }
    let Ok(rec) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if rec.get("type").and_then(|v| v.as_str()) != Some("assistant") {
        return;
    }
    let Some(msg) = rec.get("message").and_then(|v| v.as_object()) else {
        return;
    };
    let Some(u) = msg.get("usage").and_then(|v| v.as_object()) else {
        return;
    };

    let model = msg
        .get("model")
        .and_then(|v| v.as_str())
        .map(normalize_model)
        .filter(|s| !s.is_empty());
    let r_in = as_u64(u.get("input_tokens"));
    let r_out = as_u64(u.get("output_tokens"));
    let r_cw = as_u64(u.get("cache_creation_input_tokens"));
    let r_cr = as_u64(u.get("cache_read_input_tokens"));
    let cost = estimate_cost_usd(model.as_deref(), r_in, r_out, r_cr, r_cw);

    let apply = |acc: &mut AgentUsage| {
        if let Some(ref m) = model {
            acc.model = Some(m.clone());
        }
        acc.input_tokens += r_in;
        acc.output_tokens += r_out;
        acc.cache_write_tokens += r_cw;
        acc.cache_read_tokens += r_cr;
        acc.estimated_cost_usd += cost;
    };

    apply(all);
    if let Some(sid) = rec.get("sessionId").and_then(|v| v.as_str())
        && !sid.is_empty()
    {
        let bucket = per_session.entry(sid.to_string()).or_default();
        apply(bucket);
    }
}

/// Parse local Claude Code JSONL transcripts under `projects_root` for `cwd`
/// (optionally filtered to one `session_id`).
pub fn read_agent_usage_in(
    projects_root: &Path,
    cwd: &str,
    session_id: Option<&str>,
) -> AgentUsage {
    let dir = project_dir_in(projects_root, cwd);
    let as_of = now_ms();
    if !dir.is_dir() {
        return AgentUsage {
            as_of,
            ..Default::default()
        };
    }

    let files: Vec<PathBuf> = if let Some(sid) = session_id {
        if !is_safe_id(sid) {
            return AgentUsage {
                as_of,
                ..Default::default()
            };
        }
        let own = dir.join(format!("{sid}.jsonl"));
        if own.is_file() { vec![own] } else { Vec::new() }
    } else {
        fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|entry| entry.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
            .collect()
    };

    let mut total = AgentUsage {
        as_of,
        ..Default::default()
    };
    for file in files {
        let Ok(raw) = fs::read_to_string(file) else {
            continue;
        };
        let mut file_all = AgentUsage::default();
        let mut by_session = HashMap::new();
        for line in raw.lines() {
            accumulate_line(line.trim(), &mut file_all, &mut by_session);
        }
        let chosen = if let Some(sid) = session_id {
            by_session.remove(sid)
        } else {
            Some(file_all)
        };
        if let Some(u) = chosen {
            total.input_tokens += u.input_tokens;
            total.output_tokens += u.output_tokens;
            total.cache_write_tokens += u.cache_write_tokens;
            total.cache_read_tokens += u.cache_read_tokens;
            total.estimated_cost_usd += u.estimated_cost_usd;
            if u.model.is_some() {
                total.model = u.model;
            }
        }
    }
    total
}

/// Tail-read up to 256 KiB from a JSONL transcript without following symlinks,
/// returning the token sum of the most recent assistant message.
pub fn read_context_tokens(transcript_path: &Path) -> Option<u64> {
    let mut file = open_nofollow_read(transcript_path).ok()?;
    let meta = file.metadata().ok()?;
    let len = meta.len() as usize;
    if len == 0 {
        return None;
    }
    let tail_len = len.min(CONTEXT_TAIL_BYTES);
    let offset = (len - tail_len) as u64;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; tail_len];
    let n = file.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("\"usage\"") {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if rec.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(u) = rec
            .get("message")
            .and_then(|m| m.get("usage"))
            .and_then(|v| v.as_object())
        else {
            continue;
        };
        return Some(
            as_u64(u.get("input_tokens"))
                + as_u64(u.get("output_tokens"))
                + as_u64(u.get("cache_creation_input_tokens"))
                + as_u64(u.get("cache_read_input_tokens")),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transcript_usage_and_tail_context() {
        let dir = std::env::temp_dir().join(format!("md-usage-{}", now_ms()));
        let cwd = "/tmp/my.repo";
        let proj = dir.join(project_key(cwd));
        fs::create_dir_all(&proj).unwrap();
        let file = proj.join("sess-1.jsonl");
        let content = concat!(
            r#"{"type":"user","sessionId":"sess-1","message":{"role":"user","content":"hi"}}"#,
            "\n",
            r#"{"type":"assistant","sessionId":"sess-1","message":{"model":"claude-opus-4-8[1m]","usage":{"input_tokens":1000,"output_tokens":500,"cache_creation_input_tokens":200,"cache_read_input_tokens":3000}}}"#,
            "\n"
        );
        fs::write(&file, content).unwrap();

        let usage = read_agent_usage_in(&dir, cwd, Some("sess-1"));
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.cache_write_tokens, 200);
        assert_eq!(usage.cache_read_tokens, 3000);
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-8"));
        assert!(usage.estimated_cost_usd > 0.0);

        assert_eq!(read_context_tokens(&file), Some(4700));
        fs::remove_dir_all(&dir).ok();
    }
}
