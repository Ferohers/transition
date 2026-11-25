//! Git repository inspection, diffs, and worktree management (`src/main/git.ts`, `worktreeAdmin.ts`).

use md_core::config::expand_tilde;
use md_core::repo_fs::{read_file_text, safe_resolve};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchInfo {
    pub current: Option<String>,
    pub detached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusEntry {
    pub path: String,
    pub index: String,
    pub worktree: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub staged: Vec<GitStatusEntry>,
    pub unstaged: Vec<GitStatusEntry>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub time: i64,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranches {
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitAheadBehind {
    pub ahead: u32,
    pub behind: u32,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiff {
    pub ok: bool,
    pub path: String,
    pub rel_path: String,
    pub head: String,
    pub working: String,
    pub head_exists: bool,
    pub working_exists: bool,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub bare: bool,
}

async fn run_git(cwd: &str, args: &[&str]) -> Result<String, String> {
    let dir = expand_tilde(cwd);
    if !dir.is_dir() {
        return Err(format!("cwd does not exist: {}", dir.display()));
    }
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("protocol.file.allow=never")
        .args(args)
        .current_dir(&dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "git did not answer in time and was stopped".to_string())?
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git exited {}", output.status)
        } else {
            err
        })
    }
}

pub async fn is_repo(cwd: &str) -> bool {
    run_git(cwd, &["rev-parse", "--is-inside-work-tree"])
        .await
        .is_ok_and(|s| s.trim() == "true")
}

pub async fn main_repo(cwd: &str) -> Option<String> {
    let common = run_git(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await
        .ok()?;
    let p = PathBuf::from(common.trim());
    if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
        p.parent().map(|d| d.to_string_lossy().into_owned())
    } else {
        run_git(cwd, &["rev-parse", "--show-toplevel"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }
}

pub async fn get_branch(cwd: &str) -> Result<GitBranchInfo, String> {
    let out = run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let name = out.trim();
    if name == "HEAD" {
        Ok(GitBranchInfo {
            current: None,
            detached: true,
        })
    } else {
        Ok(GitBranchInfo {
            current: Some(name.to_string()),
            detached: false,
        })
    }
}

pub async fn get_status(cwd: &str) -> Result<GitStatus, String> {
    let out = run_git(cwd, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]).await?;
    let tokens: Vec<&str> = out.split('\0').filter(|t| !t.is_empty()).collect();
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok.len() < 3 {
            i += 1;
            continue;
        }
        let index = &tok[0..1];
        let worktree = &tok[1..2];
        let path = &tok[3..];
        if index == "?" && worktree == "?" {
            untracked.push(path.to_string());
            i += 1;
            continue;
        }
        let mut old_path = None;
        if (matches!(index, "R" | "C") || matches!(worktree, "R" | "C"))
            && let Some(next_tok) = tokens.get(i + 1)
        {
            old_path = Some((*next_tok).to_string());
            i += 1;
        }
        let entry = GitStatusEntry {
            path: path.to_string(),
            index: index.to_string(),
            worktree: worktree.to_string(),
            old_path,
        };
        if index != " " && index != "?" {
            staged.push(entry.clone());
        }
        if worktree != " " && worktree != "?" {
            unstaged.push(entry);
        }
        i += 1;
    }

    Ok(GitStatus {
        staged,
        unstaged,
        untracked,
    })
}

pub async fn get_log(cwd: &str, count: usize) -> Result<Vec<GitCommit>, String> {
    let max_count = format!("--max-count={}", count.clamp(1, 500));
    let fmt = "--pretty=format:%H\x1f%P\x1f%s\x1f%an\x1f%at\x1f%D\x1e";
    let out = run_git(cwd, &["log", "--all", &max_count, fmt]).await?;
    let mut commits = Vec::new();
    for rec in out.split('\x1e') {
        if rec.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = rec.split('\x1f').collect();
        if fields.len() < 6 || fields[0].is_empty() {
            continue;
        }
        let sha = fields[0].to_string();
        let short_sha = sha.chars().take(7).collect();
        let parents = fields[1]
            .split(' ')
            .filter(|p| !p.is_empty())
            .map(ToString::to_string)
            .collect();
        let refs = fields[5]
            .split(", ")
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(ToString::to_string)
            .collect();
        commits.push(GitCommit {
            sha,
            short_sha,
            parents,
            subject: fields[2].to_string(),
            author: fields[3].to_string(),
            time: fields[4].parse::<i64>().unwrap_or(0),
            refs,
        });
    }
    Ok(commits)
}

pub async fn get_branches(cwd: &str) -> Result<GitBranches, String> {
    let out = run_git(cwd, &["branch", "-a", "--format=%(HEAD)\x1f%(refname:short)"]).await?;
    let mut current = None;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in out.lines() {
        let Some((head, name)) = line.split_once('\x1f') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if head.trim() == "*" {
            current = Some(name.to_string());
        }
        if let Some(stripped) = name.strip_prefix("remotes/") {
            remote.push(stripped.to_string());
        } else {
            local.push(name.to_string());
        }
    }
    Ok(GitBranches {
        local,
        remote,
        current,
    })
}

pub async fn get_ahead_behind(cwd: &str) -> Result<GitAheadBehind, String> {
    let Ok(up) = run_git(
        cwd,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .await
    else {
        return Ok(GitAheadBehind {
            ahead: 0,
            behind: 0,
            upstream: None,
        });
    };
    let upstream = up.trim().to_string();
    let spec = format!("HEAD...{upstream}");
    let ab = run_git(cwd, &["rev-list", "--left-right", "--count", &spec]).await?;
    let mut parts = ab.trim().split('\t');
    let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok(GitAheadBehind {
        ahead,
        behind,
        upstream: Some(upstream),
    })
}

pub async fn get_diff(cwd: &str, rel_path: &str) -> Result<GitDiff, String> {
    let abs = safe_resolve(cwd, rel_path).ok_or_else(|| "path escapes repo".to_string())?;
    let spec = format!("HEAD:{rel_path}");
    let head_res = run_git(cwd, &["show", &spec]).await;
    let (head, head_exists) = match head_res {
        Ok(s) if s.len() <= MAX_DIFF_BYTES => (s, true),
        Ok(_) => return Err("HEAD file exceeds 2 MB diff limit".to_string()),
        Err(_) => (String::new(), false),
    };
    let (working, working_exists, is_binary) = if abs.exists() {
        match read_file_text(cwd, rel_path) {
            Ok((text, _, _)) => (text, true, head.contains('\0')),
            Err(e) if e.contains("binary") => (String::new(), true, true),
            Err(e) => return Err(e),
        }
    } else {
        (String::new(), false, head.contains('\0'))
    };

    Ok(GitDiff {
        ok: true,
        path: abs.to_string_lossy().into_owned(),
        rel_path: rel_path.to_string(),
        head,
        working,
        head_exists,
        working_exists,
        is_binary,
    })
}

pub async fn list_worktrees(cwd: &str) -> Result<Vec<GitWorktreeInfo>, String> {
    let out = run_git(cwd, &["worktree", "list", "--porcelain"]).await?;
    let mut list = Vec::new();
    let mut cur: Option<GitWorktreeInfo> = None;
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(done) = cur.take() {
                list.push(done);
            }
            cur = Some(GitWorktreeInfo {
                path: path.to_string(),
                branch: None,
                head: None,
                bare: false,
            });
        } else if let Some(c) = cur.as_mut() {
            if let Some(h) = line.strip_prefix("HEAD ") {
                c.head = Some(h.to_string());
            } else if let Some(b) = line.strip_prefix("branch ") {
                c.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "bare" {
                c.bare = true;
            }
        }
    }
    if let Some(done) = cur {
        list.push(done);
    }
    Ok(list)
}

pub async fn checkout(cwd: &str, reference: &str, detach: bool) -> Result<(), String> {
    if reference.starts_with('-') || reference.contains('\0') {
        return Err("invalid revision".to_string());
    }
    let mut args = vec!["checkout"];
    if detach {
        args.push("--detach");
    }
    args.push(reference);
    run_git(cwd, &args).await.map(|_| ())
}

pub fn directory_size_bytes(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            total = total.saturating_add(directory_size_bytes(&entry.path()));
        } else if ft.is_file() {
            total = total.saturating_add(entry.metadata().map_or(0, |m| m.len()));
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn git_detects_non_repo_and_repo() {
        let dir = std::env::temp_dir().join(format!(
            "md-git-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path_str = dir.to_string_lossy().into_owned();
        assert!(!is_repo(&path_str).await);

        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .await
            .unwrap();
        assert!(init.success());
        assert!(is_repo(&path_str).await);
        assert!(main_repo(&path_str).await.is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
