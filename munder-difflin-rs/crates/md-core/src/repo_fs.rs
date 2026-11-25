//! Root-confined workspace filesystem operations (`src/main/fs.ts`).

use crate::config::expand_tilde;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
const MAX_BINARY_READ_BYTES: u64 = 10 * 1024 * 1024;
const SEARCH_MAX_FILE_BYTES: u64 = 1024 * 1024;
const SEARCH_SNIFF_BYTES: usize = 8192;
const MAX_LINE_CHARS: usize = 400;

const SEARCH_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "dist",
    "out",
    "build",
    "coverage",
    ".next",
    ".nuxt",
    ".cache",
    ".turbo",
    ".parcel-cache",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    "target",
    "vendor",
    "Pods",
    ".gradle",
    ".dart_tool",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    pub max_hits: Option<usize>,
    pub time_budget_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub rel: String,
    pub line: usize,
    pub col: usize,
    pub text: String,
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub ok: bool,
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
    pub files_scanned: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn canonicalize_with_tail(real_root: &Path, abs_path: &Path) -> Option<PathBuf> {
    let mut probe = abs_path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match fs::canonicalize(&probe) {
            Ok(real) => {
                let mut full = real;
                for part in tail {
                    full.push(part);
                }
                if !full.starts_with(real_root) {
                    return None;
                }
                return Some(full);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = probe.file_name()?.to_os_string();
                let parent = probe.parent()?;
                if parent == probe {
                    return None;
                }
                tail.insert(0, name);
                probe = parent.to_path_buf();
            }
            Err(_) => return None,
        }
    }
}

fn has_symlink_component(real_root: &Path, abs_path: &Path) -> bool {
    let Ok(rel) = abs_path.strip_prefix(real_root) else {
        return true;
    };
    let mut cur = real_root.to_path_buf();
    for part in rel.components() {
        cur.push(part.as_os_str());
        match fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

/// Confines `rel` inside `root` and returns the canonical path, rejecting
/// traversals, external targets, and dangling symlinks.
pub fn safe_resolve(root: &str, rel: &str) -> Option<PathBuf> {
    let root_path = expand_tilde(root);
    let real_root = fs::canonicalize(&root_path).ok()?;
    let rel_path = Path::new(rel);
    let candidate = if rel_path.is_absolute() {
        normalize_lexical(rel_path)
    } else {
        normalize_lexical(&real_root.join(rel_path))
    };
    if !candidate.starts_with(&real_root) {
        return None;
    }
    let abs = canonicalize_with_tail(&real_root, &candidate)?;
    if has_symlink_component(&real_root, &abs) {
        return None;
    }
    Some(abs)
}

pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn open_nofollow_read(abs: &Path) -> Result<File, String> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        opts.custom_flags(libc_o_nofollow_nonblock());
    }
    let file = opts.open(abs).map_err(|e| e.to_string())?;
    if !file.metadata().map(|m| m.is_file()).unwrap_or(false) {
        return Err("target is not a regular file".to_string());
    }
    Ok(file)
}

#[cfg(unix)]
fn libc_o_nofollow_nonblock() -> i32 {
    // Portable macOS/Linux O_NOFOLLOW | O_NONBLOCK flags via standard POSIX values
    #[cfg(target_os = "macos")]
    {
        0x0100 | 0x0004
    }
    #[cfg(not(target_os = "macos"))]
    {
        0o400000 | 0o4000
    }
}

pub fn list_dir(root: &str, rel: &str) -> Result<(Vec<DirEntryInfo>, PathBuf), String> {
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    let rd = fs::read_dir(&abs).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let meta = fs::metadata(item.path()).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        let size = meta.as_ref().map_or(0, |m| m.len());
        let mtime = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        entries.push(DirEntryInfo {
            name,
            is_dir,
            size,
            mtime,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok((entries, abs))
}

pub fn read_file_text(root: &str, rel: &str) -> Result<(String, PathBuf, u64), String> {
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    let mut file = open_nofollow_read(&abs)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!(
            "file too large ({:.1} MB)",
            meta.len() as f64 / 1024.0 / 1024.0
        ));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_READ_BYTES {
        return Err("file grew past size limit".to_string());
    }
    if buf.contains(&0) {
        return Err("binary file (not displayable)".to_string());
    }
    let content = String::from_utf8(buf).map_err(|_| "file is not valid UTF-8".to_string())?;
    Ok((content, abs, meta.len()))
}

pub fn read_file_binary(root: &str, rel: &str) -> Result<(Vec<u8>, &'static str, PathBuf), String> {
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    let mut file = open_nofollow_read(&abs)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    if meta.len() > MAX_BINARY_READ_BYTES {
        return Err(format!(
            "file too large ({:.1} MB)",
            meta.len() as f64 / 1024.0 / 1024.0
        ));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_BINARY_READ_BYTES {
        return Err("file grew past size limit".to_string());
    }
    Ok((buf, mime_for_path(&abs), abs))
}

pub fn write_file_text(root: &str, rel: &str, content: &str) -> Result<PathBuf, String> {
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        #[cfg(target_os = "macos")]
        opts.custom_flags(0x0100);
        #[cfg(not(target_os = "macos"))]
        opts.custom_flags(0o400000);
    }
    use std::io::Write;
    let mut f = opts.open(&abs).map_err(|e| e.to_string())?;
    f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(abs)
}

fn bad_name(rel: &str) -> Option<&'static str> {
    if rel.trim().is_empty() {
        return Some("name is empty");
    }
    if rel.contains('\0') {
        return Some("name contains a null byte");
    }
    if rel.len() > 1024 {
        return Some("name is too long");
    }
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Some("name is empty");
    }
    for p in parts {
        if p == "." || p == ".." {
            return Some("name contains a path segment that is not a name");
        }
        if p.trim() != p {
            return Some("name starts or ends with a space");
        }
    }
    None
}

pub fn make_dir_in(root: &str, rel: &str) -> Result<PathBuf, String> {
    if let Some(bad) = bad_name(rel) {
        return Err(bad.to_string());
    }
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    fs::create_dir(&abs).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => "already exists".to_string(),
        std::io::ErrorKind::NotFound => "the folder above it does not exist".to_string(),
        _ => e.to_string(),
    })?;
    Ok(abs)
}

pub fn create_file_in(root: &str, rel: &str) -> Result<PathBuf, String> {
    if let Some(bad) = bad_name(rel) {
        return Err(bad.to_string());
    }
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&abs)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => "already exists".to_string(),
            std::io::ErrorKind::NotFound => "the folder above it does not exist".to_string(),
            _ => e.to_string(),
        })?;
    Ok(abs)
}

pub fn rename_in(root: &str, from_rel: &str, to_rel: &str) -> Result<PathBuf, String> {
    if let Some(bad) = bad_name(from_rel).or_else(|| bad_name(to_rel)) {
        return Err(bad.to_string());
    }
    let from = safe_resolve(root, from_rel).ok_or_else(|| "path escapes root".to_string())?;
    let to = safe_resolve(root, to_rel).ok_or_else(|| "path escapes root".to_string())?;
    if to.exists() {
        return Err("destination already exists".to_string());
    }
    fs::rename(&from, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

pub fn trash_in(root: &str, rel: &str) -> Result<PathBuf, String> {
    if let Some(bad) = bad_name(rel) {
        return Err(bad.to_string());
    }
    let real_root =
        fs::canonicalize(expand_tilde(root)).map_err(|e| format!("invalid root: {e}"))?;
    let abs = safe_resolve(root, rel).ok_or_else(|| "path escapes root".to_string())?;
    if abs == real_root {
        return Err("refusing to trash workspace root".to_string());
    }
    trash::delete(&abs).map_err(|e| e.to_string())?;
    Ok(abs)
}

pub fn search_in_root(root: &str, query: &str, opts: &SearchOptions) -> SearchResult {
    let Some(abs_root) = safe_resolve(root, "") else {
        return SearchResult {
            ok: false,
            hits: Vec::new(),
            truncated: false,
            files_scanned: 0,
            error: Some("path escapes root".to_string()),
        };
    };
    if query.is_empty() {
        return SearchResult {
            ok: true,
            hits: Vec::new(),
            truncated: false,
            files_scanned: 0,
            error: None,
        };
    }

    let raw_pat = if opts.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pat = if opts.whole_word {
        format!(r"\b(?:{raw_pat})\b")
    } else {
        raw_pat
    };
    let re = match RegexBuilder::new(&pat)
        .case_insensitive(!opts.case_sensitive)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            return SearchResult {
                ok: false,
                hits: Vec::new(),
                truncated: false,
                files_scanned: 0,
                error: Some(e.to_string()),
            };
        }
    };

    let max_hits = opts.max_hits.unwrap_or(500).clamp(1, 5000);
    let budget = Duration::from_millis(opts.time_budget_ms.unwrap_or(5000).clamp(250, 30_000));
    let deadline = Instant::now() + budget;

    let mut hits = Vec::new();
    let mut files_scanned = 0usize;
    let mut truncated = false;

    walk_search(
        &abs_root,
        "",
        &re,
        max_hits,
        deadline,
        &mut hits,
        &mut files_scanned,
        &mut truncated,
    );

    SearchResult {
        ok: true,
        hits,
        truncated,
        files_scanned,
        error: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_search(
    dir_abs: &Path,
    dir_rel: &str,
    re: &regex::Regex,
    max_hits: usize,
    deadline: Instant,
    hits: &mut Vec<SearchHit>,
    files_scanned: &mut usize,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    let Ok(entries) = fs::read_dir(dir_abs) else {
        return;
    };
    for entry in entries.flatten() {
        if *truncated || Instant::now() > deadline {
            *truncated = true;
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let rel = if dir_rel.is_empty() {
            name.clone()
        } else {
            format!("{dir_rel}/{name}")
        };
        if ft.is_dir() {
            if SEARCH_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk_search(
                &entry.path(),
                &rel,
                re,
                max_hits,
                deadline,
                hits,
                files_scanned,
                truncated,
            );
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() > SEARCH_MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let sniff_len = bytes.len().min(SEARCH_SNIFF_BYTES);
        if bytes[..sniff_len].contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        *files_scanned += 1;
        for (line_idx, line) in text.lines().enumerate() {
            for m in re.find_iter(line) {
                let preview: String = if line.chars().count() > MAX_LINE_CHARS {
                    let mut s: String = line.chars().take(MAX_LINE_CHARS).collect();
                    s.push('…');
                    s
                } else {
                    line.to_string()
                };
                hits.push(SearchHit {
                    rel: rel.clone(),
                    line: line_idx + 1,
                    col: line[..m.start()].chars().count() + 1,
                    text: preview,
                    length: m.as_str().chars().count(),
                });
                if hits.len() >= max_hits {
                    *truncated = true;
                    return;
                }
            }
        }
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_confinement_and_crud_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-fs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let root = dir.to_string_lossy().into_owned();

        assert!(safe_resolve(&root, "../etc/passwd").is_none());
        create_file_in(&root, "hello.txt").unwrap();
        assert!(create_file_in(&root, "hello.txt").is_err());
        write_file_text(&root, "hello.txt", "find me in rust").unwrap();

        let (txt, _, _) = read_file_text(&root, "hello.txt").unwrap();
        assert_eq!(txt, "find me in rust");

        let res = search_in_root(&root, "rust", &SearchOptions {
            regex: false,
            case_sensitive: false,
            whole_word: true,
            max_hits: None,
            time_budget_ms: None,
        });
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].col, 12);

        make_dir_in(&root, "sub").unwrap();
        rename_in(&root, "hello.txt", "sub/hello.txt").unwrap();
        assert!(read_file_text(&root, "sub/hello.txt").is_ok());

        fs::remove_dir_all(&dir).ok();
    }
}
