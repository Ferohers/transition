use crate::state::{AppState, IdeState};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use md_core::repo_fs::{
    SearchOptions, create_file_in, is_safe_id, list_dir, make_dir_in, read_file_binary,
    read_file_text, rename_in, search_in_root, trash_in, write_file_text,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootRelReq {
    root: String,
    #[serde(default)]
    rel: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteFileReq {
    root: String,
    rel: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchReq {
    root: String,
    query: String,
    #[serde(default)]
    opts: Option<SearchOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameReq {
    root: String,
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CwdQuery {
    cwd: String,
    n: Option<usize>,
    skip: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitCommitFilesQuery {
    cwd: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitShowFileQuery {
    cwd: String,
    rev: String,
    rel_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitCompareRefsQuery {
    cwd: String,
    base: String,
    head: String,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitDiffQuery {
    cwd: String,
    rel_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitCheckoutReq {
    cwd: String,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    detach: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathReq {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdeDirtyReq {
    unsaved: usize,
    #[serde(default)]
    open_files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdeOpenReq {
    root: String,
    rel: String,
}

async fn fs_list(Json(req): Json<RootRelReq>) -> Json<Value> {
    match list_dir(&req.root, &req.rel) {
        Ok((entries, path)) => Json(serde_json::json!({ "ok": true, "entries": entries, "path": path })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_read(Json(req): Json<RootRelReq>) -> Json<Value> {
    match read_file_text(&req.root, &req.rel) {
        Ok((content, path, size)) => {
            Json(serde_json::json!({ "ok": true, "content": content, "path": path, "size": size }))
        }
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_read_binary(Json(req): Json<RootRelReq>) -> Json<Value> {
    match read_file_binary(&req.root, &req.rel) {
        Ok((bytes, mime, path)) => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Json(serde_json::json!({
                "ok": true,
                "base64": b64,
                "mime": mime,
                "path": path,
                "size": bytes.len()
            }))
        }
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_write(Json(req): Json<WriteFileReq>) -> Json<Value> {
    match write_file_text(&req.root, &req.rel, &req.content) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_search(Json(req): Json<SearchReq>) -> Json<Value> {
    let opts = req.opts.unwrap_or(SearchOptions {
        regex: false,
        case_sensitive: false,
        whole_word: false,
        max_hits: None,
        time_budget_ms: None,
    });
    let res = search_in_root(&req.root, &req.query, &opts);
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
}

async fn fs_mkdir(Json(req): Json<RootRelReq>) -> Json<Value> {
    match make_dir_in(&req.root, &req.rel) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path, "rel": req.rel })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_create(Json(req): Json<RootRelReq>) -> Json<Value> {
    match create_file_in(&req.root, &req.rel) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path, "rel": req.rel })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_rename(Json(req): Json<RenameReq>) -> Json<Value> {
    match rename_in(&req.root, &req.from, &req.to) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path, "rel": req.to })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn fs_trash(Json(req): Json<RootRelReq>) -> Json<Value> {
    match trash_in(&req.root, &req.rel) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path, "rel": req.rel })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn git_is_repo(Query(q): Query<CwdQuery>) -> Json<bool> {
    Json(md_git::is_repo(&q.cwd).await)
}

async fn git_main_repo(Query(q): Query<CwdQuery>) -> Json<Option<String>> {
    Json(md_git::main_repo(&q.cwd).await)
}

async fn git_branch(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_branch(&q.cwd).await {
        Ok(info) => Json(serde_json::to_value(info).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_status(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_status(&q.cwd).await {
        Ok(st) => Json(serde_json::to_value(st).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_log(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_log(&q.cwd, q.n.unwrap_or(50)).await {
        Ok(commits) => Json(serde_json::to_value(commits).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_branches(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_branches(&q.cwd).await {
        Ok(b) => Json(serde_json::to_value(b).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_ahead_behind(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_ahead_behind(&q.cwd).await {
        Ok(ab) => Json(serde_json::to_value(ab).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_diff(Query(q): Query<GitDiffQuery>) -> Json<Value> {
    match md_git::get_diff(&q.cwd, &q.rel_path).await {
        Ok(diff) => Json(serde_json::to_value(diff).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn git_worktrees(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::list_worktrees(&q.cwd).await {
        Ok(list) => Json(serde_json::to_value(list).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_checkout(Json(req): Json<GitCheckoutReq>) -> Json<Value> {
    match md_git::checkout(&req.cwd, &req.reference, req.detach).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(error) => Json(serde_json::json!({ "ok": false, "error": error })),
    }
}

async fn git_log_graph(Query(q): Query<CwdQuery>) -> Json<Value> {
    match md_git::get_log_graph(&q.cwd, q.n.unwrap_or(200), q.skip.unwrap_or(0)).await {
        Ok(commits) => Json(serde_json::to_value(commits).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_commit_files(Query(q): Query<GitCommitFilesQuery>) -> Json<Value> {
    match md_git::get_commit_files(&q.cwd, &q.sha).await {
        Ok(files) => Json(serde_json::to_value(files).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn git_show_file(Query(q): Query<GitShowFileQuery>) -> Json<Value> {
    let res = md_git::get_file_at_rev(&q.cwd, &q.rev, &q.rel_path).await;
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
}

async fn git_compare_refs(Query(q): Query<GitCompareRefsQuery>) -> Json<Value> {
    let mode = q.mode.as_deref().unwrap_or("three");
    match md_git::compare_refs(&q.cwd, &q.base, &q.head, mode).await {
        Ok(cmp) => Json(serde_json::to_value(cmp).unwrap_or(Value::Null)),
        Err(error) => Json(serde_json::json!({ "error": error })),
    }
}

async fn fs_folder_state(Json(req): Json<PathReq>) -> Json<Value> {
    let p = md_core::config::expand_tilde(&req.path);
    if !p.is_dir() {
        return Json(serde_json::json!({
            "exists": false,
            "empty": true,
            "path": p.to_string_lossy()
        }));
    }
    let empty = std::fs::read_dir(&p)
        .map(|mut rd| rd.next().is_none())
        .unwrap_or(true);
    Json(serde_json::json!({
        "exists": true,
        "empty": empty,
        "path": p.to_string_lossy()
    }))
}

async fn worktrees_list(State(state): State<AppState>) -> Json<Value> {
    let Some(home) = state.harness_home() else {
        return Json(serde_json::json!([]));
    };
    let wt_root = home.join("worktrees");
    let Ok(rd) = std::fs::read_dir(&wt_root) else {
        return Json(serde_json::json!([]));
    };
    let mut rows = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            rows.push(serde_json::json!({
                "name": entry.file_name().to_string_lossy(),
                "path": p.to_string_lossy()
            }));
        }
    }
    Json(Value::Array(rows))
}

async fn worktrees_size(Json(req): Json<PathReq>) -> Json<Value> {
    let p = Path::new(&req.path);
    if !p.is_dir() {
        return Json(Value::Null);
    }
    Json(serde_json::json!(md_git::directory_size_bytes(p)))
}

async fn fs_stat_abs(Json(req): Json<PathReq>) -> Json<Value> {
    let p = md_core::config::expand_tilde(&req.path);
    match std::fs::metadata(&p) {
        Ok(m) => Json(serde_json::json!({
            "exists": true,
            "isFile": m.is_file(),
            "isDir": m.is_dir(),
            "path": p.to_string_lossy()
        })),
        Err(_) => Json(serde_json::json!({
            "exists": false,
            "isFile": false,
            "isDir": false,
            "path": p.to_string_lossy()
        })),
    }
}

fn same_or_inside_path(candidate: &Path, root: &Path) -> bool {
    if candidate == root || candidate.starts_with(root) {
        return true;
    }
    if let (Ok(cc), Ok(cr)) = (std::fs::canonicalize(candidate), std::fs::canonicalize(root)) {
        return cc == cr || cc.starts_with(&cr);
    }
    false
}

/// Returns `true` when `path` is app-internal storage, a harness floor home
/// (e.g. `~/HarnessAgents2`), or a non-project internal directory inside one.
fn is_app_or_floor_dir(
    path: &Path,
    cfg: &md_core::config::HarnessConfig,
    data_dir: &Path,
) -> bool {
    let canon_p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(ref raw) = cfg.harness_home {
        let p = md_core::config::expand_tilde(raw);
        if !p.as_os_str().is_empty() {
            homes.push(p);
        }
    }
    for raw in &cfg.recent_hives {
        let p = md_core::config::expand_tilde(raw);
        if !p.as_os_str().is_empty() && !homes.contains(&p) {
            homes.push(p);
        }
    }

    for h in homes {
        let canon_h = std::fs::canonicalize(&h).unwrap_or(h);
        if canon_p == canon_h {
            return true;
        }
        let hive_dir = canon_h.join("hive");
        let projects_dir = canon_h.join("projects");
        let hive_projects_dir = hive_dir.join("projects");
        if canon_p == hive_dir || canon_p == projects_dir || canon_p == hive_projects_dir {
            return true;
        }
        if same_or_inside_path(&canon_p, &canon_h.join("worktrees"))
            || same_or_inside_path(&canon_p, &canon_h.join("roster-backups"))
            || same_or_inside_path(&canon_p, &canon_h.join(".palace-stage"))
        {
            return true;
        }
        if md_hive::manager::in_hive_projects(&hive_dir, &canon_p)
            || same_or_inside_path(&canon_p, &projects_dir)
        {
            return false;
        }
        if same_or_inside_path(&canon_p, &hive_dir) {
            return true;
        }
    }

    if same_or_inside_path(path, data_dir) {
        return true;
    }

    // Heuristic catch for unlisted floor directories (e.g. other floors on disk) or agent dirs.
    let hive_sub = path.join("hive");
    if path.join("roster.json").is_file()
        && (hive_sub.join("registry.json").is_file() || hive_sub.join("PROTOCOL.md").is_file())
    {
        return true;
    }
    if path.join("identity.md").is_file()
        && path.join("inbox").is_dir()
        && path.join("outbox").is_dir()
    {
        return true;
    }
    false
}

fn suggested_project_root() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    for rel in ["Projects", "Developer", "Code", "Workspaces", "Documents", "Desktop"] {
        let p = home.join(rel);
        if p.is_dir() {
            return Some(p);
        }
    }
    Some(home)
}

fn discover_machine_projects(
    cfg: &md_core::config::HarnessConfig,
    data_dir: &Path,
    push_unique: &mut impl FnMut(PathBuf),
) {
    let Some(user_home) = dirs::home_dir() else {
        return;
    };
    const CODE_ROOTS: &[&str] = &[
        "Projects",
        "Developer",
        "Code",
        "Workspaces",
        "Workspace",
        "Repos",
        "src",
        "GitHub",
        "Sites",
    ];
    let mut found = 0usize;
    let max_discovered = 50usize;

    for rel in CODE_ROOTS {
        if found >= max_discovered {
            break;
        }
        let root = user_home.join(rel);
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut children: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('.') || n == "node_modules")
                    && !is_app_or_floor_dir(p, cfg, data_dir)
            })
            .collect();
        children.sort();
        for c in children {
            if found >= max_discovered {
                break;
            }
            push_unique(c);
            found += 1;
        }
    }

    // In general user folders (Documents, Desktop, ~), only surface directories that are git repos.
    for root in [user_home.join("Documents"), user_home.join("Desktop"), user_home.clone()] {
        if found >= max_discovered {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut children: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.join(".git").exists()
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with('.') || n == "Library" || n == "Applications")
                    && !is_app_or_floor_dir(p, cfg, data_dir)
            })
            .collect();
        children.sort();
        for c in children {
            if found >= max_discovered {
                break;
            }
            push_unique(c);
            found += 1;
        }
    }
}

fn collect_real_project_paths(
    state: &AppState,
    include_machine_discovery: bool,
) -> (Option<PathBuf>, Vec<PathBuf>) {
    let cfg = state.config.lock().unwrap().read();
    let home = cfg
        .harness_home
        .as_deref()
        .map(md_core::config::expand_tilde)
        .filter(|p| !p.as_os_str().is_empty());

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if p.is_dir()
            && !is_app_or_floor_dir(&p, &cfg, &state.data_dir)
            && !paths.contains(&p)
        {
            paths.push(p);
        }
    };

    for r in &cfg.registered_repos {
        let exp = md_core::config::expand_tilde(r);
        let resolved = md_git::main_repo_sync(&exp).unwrap_or(exp);
        push_unique(resolved);
    }

    if let Some(snap) = state.roster.lock().unwrap().read(home.as_deref()) {
        for agent in snap.agents {
            let raw = agent
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if raw.is_empty() {
                continue;
            }
            let exp = md_core::config::expand_tilde(raw);
            let resolved = md_git::main_repo_sync(&exp).unwrap_or(exp);
            push_unique(resolved);
        }
    }

    if let Some(ref h) = home {
        for base in [h.join("projects"), h.join("hive").join("projects")] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                let mut children: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir()
                            && !p
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.starts_with('.'))
                    })
                    .collect();
                children.sort();
                for c in children {
                    push_unique(c);
                }
            }
        }
    }

    if include_machine_discovery {
        discover_machine_projects(&cfg, &state.data_dir, &mut push_unique);
    }

    (home, paths)
}

async fn workspaces_list(State(state): State<AppState>) -> Json<Value> {
    let (home, paths) = collect_real_project_paths(&state, true);
    let folders: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    Json(json!({
        "ok": true,
        "harnessHome": home.map(|p| p.to_string_lossy().into_owned()),
        "suggestedRoot": suggested_project_root().map(|p| p.to_string_lossy().into_owned()),
        "folders": folders,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowseQuery {
    path: Option<String>,
}

async fn workspaces_browse(
    State(state): State<AppState>,
    Query(q): Query<BrowseQuery>,
) -> Json<Value> {
    let cfg = state.config.lock().unwrap().read();
    let target = q
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(md_core::config::expand_tilde)
        .or_else(suggested_project_root)
        .unwrap_or_else(|| PathBuf::from("/"));

    if !target.is_dir() {
        return Json(json!({
            "ok": false,
            "error": format!("folder does not exist: {}", target.display()),
            "path": target.to_string_lossy(),
            "dirs": []
        }));
    }

    let user_home = dirs::home_dir();
    let at_user_home = user_home.as_ref().is_some_and(|uh| same_or_inside_path(&target, uh) && target == *uh);
    let Ok(rd) = std::fs::read_dir(&target) else {
        return Json(json!({
            "ok": false,
            "error": format!("cannot read folder: {}", target.display()),
            "path": target.to_string_lossy(),
            "dirs": []
        }));
    };

    let mut entries: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            p.is_dir()
                && !name.starts_with('.')
                && !(at_user_home && matches!(name, "Library" | "Applications"))
                && !is_app_or_floor_dir(p, &cfg, &state.data_dir)
        })
        .collect();
    entries.sort();

    let dirs: Vec<Value> = entries
        .into_iter()
        .take(200)
        .map(|p| {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let is_git = p.join(".git").exists();
            json!({
                "name": name,
                "path": p.to_string_lossy(),
                "isGit": is_git,
            })
        })
        .collect();

    let parent = target.parent().map(|p| p.to_string_lossy().into_owned());
    Json(json!({
        "ok": true,
        "path": target.to_string_lossy(),
        "parent": parent,
        "dirs": dirs,
    }))
}

pub(crate) fn build_projects_payload(state: &AppState) -> Value {
    let (_home, paths) = collect_real_project_paths(state, false);

    let projects: Vec<Value> = paths
        .into_iter()
        .map(|p| {
            let p_str = p.to_string_lossy().into_owned();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&p_str)
                .to_string();
            let is_git = md_git::is_repo_sync(&p);
            json!({
                "key": p_str,
                "name": name,
                "local": { "path": p_str, "git": is_git },
                "cloud": Value::Null,
            })
        })
        .collect();
    json!({ "projects": projects })
}

async fn projects_list(State(state): State<AppState>) -> Json<Value> {
    Json(build_projects_payload(&state))
}

async fn workspaces_ensure(
    State(state): State<AppState>,
    Json(req): Json<PathReq>,
) -> Json<Value> {
    let raw = req.path.trim();
    if raw.is_empty() {
        return Json(json!({ "ok": false, "error": "folder path cannot be empty" }));
    }
    let home = state.harness_home();
    let resolved = if !raw.starts_with('/')
        && !raw.starts_with('~')
        && !(raw.len() >= 2 && raw.as_bytes()[1] == b':')
    {
        let Some(ref h) = home else {
            return Json(json!({ "ok": false, "error": "no harnessHome configured for relative path" }));
        };
        if let Some(joined) = md_core::repo_fs::safe_resolve(&h.to_string_lossy(), raw) {
            joined
        } else {
            return Json(json!({ "ok": false, "error": "invalid subfolder path" }));
        }
    } else {
        md_core::config::expand_tilde(raw)
    };

    if resolved.is_dir() {
        return Json(json!({
            "ok": true,
            "path": resolved.to_string_lossy(),
            "created": false,
        }));
    }

    let allowed_to_create = home.as_ref().is_some_and(|h| {
        let canon_h = std::fs::canonicalize(h).unwrap_or_else(|_| h.clone());
        resolved.starts_with(h) || resolved.starts_with(&canon_h)
    });
    if !allowed_to_create {
        return Json(json!({
            "ok": false,
            "error": format!("folder does not exist: {} (pick or name a folder inside harnessHome to create it)", resolved.display()),
        }));
    }
    match std::fs::create_dir_all(&resolved) {
        Ok(()) => {
            if let Some(ref h) = home {
                let hive_root = h.join("hive");
                if md_hive::manager::in_hive_projects(&hive_root, &resolved)
                    && let Some(top) = md_hive::manager::hive_project_top_of(&hive_root, &resolved)
                {
                    let _ = md_git::init_repo_sync(&top, Some("Project folder made by Munder Difflin"));
                }
            }
            state.emit("projects:changed", build_projects_payload(&state));
            Json(json!({
                "ok": true,
                "path": resolved.to_string_lossy(),
                "created": true,
            }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn tools_status(State(state): State<AppState>) -> Json<Value> {
    let platform = md_core::host_node_platform();
    let is_win = platform == "win32";
    let git_posix = if platform == "darwin" {
        "brew install git || xcode-select --install"
    } else {
        "sudo apt install git   # or: sudo dnf install git / pacman -S git"
    };

    let mut rows: Vec<Value> = Vec::new();
    let home = state.harness_home();

    // Base prerequisites (matching upstream `src/shared/toolCatalog.ts`)
    let uv_path = md_pty::manager::resolve_command("uv");
    rows.push(json!({
        "id": "uv",
        "bin": "uv",
        "label": "uv",
        "kind": "prerequisite",
        "why": "Installs and runs mempalace. A self-contained Python toolchain — it does not touch any Python you already have.",
        "essential": true,
        "found": uv_path.as_ref().is_some_and(|p| p.is_file()),
        "path": uv_path.map(|p| p.to_string_lossy().into_owned()),
        "installCommand": if is_win {
            "powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\""
        } else {
            "curl -LsSf https://astral.sh/uv/install.sh | sh"
        },
        "docsUrl": "https://docs.astral.sh/uv/",
    }));

    let mem_bin = md_pty::manager::resolve_command("mempalace");
    let mem_stat = md_hive::memory::memory_status(home.as_deref(), true, None);
    let mem_found = mem_bin.as_ref().is_some_and(|p| p.is_file());
    let mem_detail = if mem_found {
        Some(if mem_stat.initialized {
            "palace initialised"
        } else {
            "installed — palace not built yet"
        })
    } else {
        None
    };
    rows.push(json!({
        "id": "mempalace",
        "bin": Value::Null,
        "label": "MemPalace — semantic memory",
        "kind": "memory",
        "why": "Meaning-based recall across everything your agents have learned. Without it they still keep plain markdown notes, but cannot search them by meaning.",
        "essential": true,
        "found": mem_found,
        "path": mem_bin.map(|p| p.to_string_lossy().into_owned()),
        "installCommand": "uv tool install mempalace==3.3.5 --python 3.10",
        "pinnedVersion": "3.3.5",
        "switchCommand": "uv tool install mempalace==3.3.5 --python 3.10 --force",
        "detail": mem_detail,
        "note": "Needs uv first.",
    }));

    let git_path = md_pty::manager::resolve_command("git");
    rows.push(json!({
        "id": "git",
        "bin": "git",
        "label": "git",
        "kind": "prerequisite",
        "why": "Worktrees let agents work in parallel without fighting over one checkout, and the hive keeps its own history in git.",
        "essential": true,
        "found": git_path.as_ref().is_some_and(|p| p.is_file()),
        "path": git_path.map(|p| p.to_string_lossy().into_owned()),
        "installCommand": if is_win { "winget install --id Git.Git -e" } else { git_posix },
        "docsUrl": "https://git-scm.com/downloads",
    }));

    let node_path = md_pty::manager::resolve_command("node");
    rows.push(json!({
        "id": "node",
        "bin": "node",
        "label": "Node.js",
        "kind": "prerequisite",
        "why": "Runs the npm-installed agent engines (OpenCode, and Claude Code on machines without the native build).",
        "essential": false,
        "found": node_path.as_ref().is_some_and(|p| p.is_file()),
        "path": node_path.map(|p| p.to_string_lossy().into_owned()),
        "installCommand": "",
        "note": "Runs npm-installed agent CLIs. On Linux/macOS install Node.js 20+ via package manager or nvm if needed.",
        "docsUrl": "https://nodejs.org",
    }));

    // Engine CLIs (matching `AGENT_PROVIDER_PRESETS` in `toolCatalog.ts`)
    let engines: &[(&str, &str, &str, bool, &str, &str, &str)] = &[
        ("claude", "claude", "Claude Code", true, "curl -fsSL https://claude.ai/install.sh | bash", "irm https://claude.ai/install.ps1 | iex", "https://docs.claude.com/en/docs/claude-code"),
        ("codex", "codex", "Codex · GPT", false, "npm install -g @openai/codex", "npm install -g @openai/codex", "https://github.com/openai/codex"),
        ("grok", "grok", "Grok · xAI", false, "curl -fsSL https://x.ai/install.sh | sh", "npm install -g @xai-official/grok-cli", "https://x.ai/cli"),
        ("kimi", "kimi", "Kimi Code", false, "curl -L code.kimi.com/install.sh | bash", "Invoke-RestMethod https://code.kimi.com/install.ps1 | Invoke-Expression", "https://www.kimi.com/code/docs/en/"),
        ("gemini", "gemini", "Gemini CLI", false, "npm install -g @google/gemini-cli", "npm install -g @google/gemini-cli", "https://github.com/google-gemini/gemini-cli"),
        ("antigravity", "agy", "Antigravity · Gemini", false, "npm install -g @anthropic-ai/antigravity", "npm install -g @anthropic-ai/antigravity", "https://antigravity.google/docs/cli"),
        ("qwen", "qwen", "Qwen (local available)", false, "npm install -g @qwen-code/qwen-code@latest", "npm install -g @qwen-code/qwen-code@latest", "https://github.com/QwenLM/qwen-code"),
        ("opencode", "opencode", "OpenCode", false, "curl -fsSL https://opencode.ai/install | bash", "npm install -g opencode-ai@latest", "https://opencode.ai/docs"),
        ("crush", "crush", "Crush · Charm", false, "npm install -g @charmland/crush", "npm install -g @charmland/crush", "https://github.com/charmbracelet/crush"),
        ("pi", "pi", "Pi", false, "npm install -g --ignore-scripts @earendil-works/pi-coding-agent", "npm install -g --ignore-scripts @earendil-works/pi-coding-agent", "https://pi.dev/docs/latest"),
        ("copilot", "copilot", "Copilot", false, "npm install -g @github/copilot", "npm install -g @github/copilot", "https://docs.github.com/copilot/concepts/agents/about-copilot-cli"),
        ("cursor", "agent", "Cursor", false, "curl https://cursor.com/install -fsS | bash", "irm 'https://cursor.com/install?win32=true' | iex", "https://cursor.com/docs/cli/install"),
    ];

    for &(id, bin, label, essential, posix_cmd, win_cmd, docs) in engines {
        let resolved = if id == "cursor" {
            md_pty::manager::resolve_command("cursor-agent")
                .or_else(|| md_pty::manager::resolve_command("agent"))
        } else {
            md_pty::manager::resolve_command(bin)
        };
        let exists = resolved.as_ref().is_some_and(|p| p.is_file());
        rows.push(json!({
            "id": format!("engine:{id}"),
            "bin": bin,
            "label": label,
            "kind": "engine",
            "why": format!("Agent engine — {bin}."),
            "essential": essential,
            "found": exists,
            "path": resolved.map(|p| p.to_string_lossy().into_owned()),
            "installCommand": if is_win { win_cmd } else { posix_cmd },
            "docsUrl": docs,
        }));
    }

    Json(Value::Array(rows))
}

async fn ide_state(State(state): State<AppState>) -> Json<IdeState> {
    Json(state.ide.lock().unwrap().clone())
}

async fn ide_dirty(State(state): State<AppState>, Json(req): Json<IdeDirtyReq>) -> Json<Value> {
    {
        let mut ide = state.ide.lock().unwrap();
        ide.unsaved_count = req.unsaved;
        ide.open_files = req.open_files.clone();
    }
    state.emit(
        "ide:dirty",
        serde_json::json!({ "unsaved": req.unsaved, "openFiles": req.open_files }),
    );
    Json(serde_json::json!({ "ok": true }))
}

async fn ide_open(State(state): State<AppState>, Json(req): Json<IdeOpenReq>) -> Json<Value> {
    state.emit(
        "ide:open",
        serde_json::json!({ "root": req.root, "rel": req.rel }),
    );
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeWorkReq {
    path: String,
    base_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeRemoveReq {
    path: String,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    delete_branch: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsUploadReq {
    name: String,
    base64: String,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsRawQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubIssuesQuery {
    repo: Option<String>,
    cwd: Option<String>,
}

fn allowed_fs_roots(state: &AppState) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(h) = state.harness_home() {
        roots.push(h);
    }
    roots.push(state.data_dir.clone());
    let cfg = state.config.lock().unwrap().read();
    for r in &cfg.registered_repos {
        roots.push(md_core::config::expand_tilde(r));
    }
    roots
}

fn is_within_allowed_roots(state: &AppState, raw_path: &str) -> Option<PathBuf> {
    let expanded = md_core::config::expand_tilde(raw_path);
    let canon = std::fs::canonicalize(&expanded).ok()?;
    for root in allowed_fs_roots(state) {
        if let Ok(root_canon) = std::fs::canonicalize(&root)
            && canon.starts_with(&root_canon)
        {
            return Some(canon);
        }
    }
    None
}

async fn worktrees_work(Json(req): Json<WorktreeWorkReq>) -> Json<Value> {
    let wt_path = md_core::config::expand_tilde(&req.path);
    let wt_str = wt_path.to_string_lossy().into_owned();
    if !wt_path.exists() {
        return Json(json!({
            "known": false,
            "uncommitted": 0,
            "unmerged": 0,
            "base": req.base_branch.unwrap_or_else(|| "main".to_string())
        }));
    }
    let st = md_git::get_status(&wt_str).await.ok();
    let uncommitted = st
        .as_ref()
        .map(|s| s.staged.len() + s.unstaged.len() + s.untracked.len())
        .unwrap_or(0);
    let ab = md_git::get_ahead_behind(&wt_str).await.ok();
    let unmerged = ab.as_ref().map(|x| x.ahead).unwrap_or(0);
    let base = req
        .base_branch
        .or_else(|| ab.and_then(|x| x.upstream))
        .unwrap_or_else(|| "main".to_string());
    Json(json!({
        "known": true,
        "uncommitted": uncommitted,
        "unmerged": unmerged,
        "base": base,
    }))
}

async fn worktrees_remove(
    State(state): State<AppState>,
    Json(req): Json<WorktreeRemoveReq>,
) -> Json<Value> {
    let wt_path = md_core::config::expand_tilde(&req.path);
    let wt_str = wt_path.to_string_lossy().into_owned();
    let allowed = is_within_allowed_roots(&state, &wt_str).is_some()
        || wt_str.contains("/worktrees/")
        || wt_str.contains("/.worktrees/");
    if !allowed {
        return Json(json!({ "ok": false, "error": "path is outside allowed workspace roots" }));
    }
    if !wt_path.exists() {
        return Json(json!({ "ok": true, "alreadyMissing": true }));
    }
    if !req.confirmed
        && let Ok(st) = md_git::get_status(&wt_str).await
    {
        let dirty = st.staged.len() + st.unstaged.len() + st.untracked.len();
        if dirty > 0 {
            return Json(json!({
                "ok": false,
                "dirty": true,
                "uncommitted": dirty,
                "error": "worktree has uncommitted changes"
            }));
        }
    }
    let branch = md_git::get_branch(&wt_str).await.ok().and_then(|b| b.current);
    let main = md_git::main_repo(&wt_str)
        .await
        .unwrap_or_else(|| wt_str.clone());
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(&main).arg("worktree").arg("remove");
    if req.confirmed {
        cmd.arg("--force");
    }
    cmd.arg(&wt_str);
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            if req.delete_branch
                && let Some(br) = branch
                && br != "main"
                && br != "master"
            {
                let _ = tokio::process::Command::new("git")
                    .args(["-C", &main, "branch", "-D", &br])
                    .output()
                    .await;
            }
            Json(json!({ "ok": true }))
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Json(json!({ "ok": false, "error": if err.is_empty() { "git worktree remove failed".to_string() } else { err } }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn fs_upload(State(state): State<AppState>, Json(req): Json<FsUploadReq>) -> Json<Value> {
    const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
    let b64 = req
        .base64
        .split_once(";base64,")
        .map(|(_, rest)| rest)
        .unwrap_or(req.base64.as_str())
        .trim();
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
        return Json(json!({ "ok": false, "error": "invalid base64 payload" }));
    };
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Json(json!({ "ok": false, "error": "file exceeds 25 MB limit" }));
    }
    let raw_name = Path::new(&req.name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment.bin");
    let clean_name: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let final_name = if clean_name.is_empty() {
        "attachment.bin".to_string()
    } else {
        clean_name
    };
    let sub = req.scope.as_deref().filter(|s| is_safe_id(s)).unwrap_or("uploads");
    let dir = state
        .harness_home()
        .map(|h| h.join("hive").join(sub))
        .unwrap_or_else(|| state.data_dir.join(sub));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Json(json!({ "ok": false, "error": e.to_string() }));
    }
    let unique = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let dest = dir.join(format!("{unique}-{final_name}"));
    if let Err(e) = std::fs::write(&dest, &bytes) {
        return Json(json!({ "ok": false, "error": e.to_string() }));
    }
    let path_str = dest.to_string_lossy().into_owned();
    Json(json!({
        "ok": true,
        "file": path_str,
        "path": path_str,
        "name": final_name,
        "bytes": bytes.len(),
    }))
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("md" | "txt" | "log" | "csv") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn fs_raw(State(state): State<AppState>, Query(q): Query<FsRawQuery>) -> Response {
    let Some(canon) = is_within_allowed_roots(&state, &q.path) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(bytes) = tokio::fs::read(&canon).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut resp = Response::new(Body::from(bytes));
    if let Ok(val) = HeaderValue::from_str(content_type_for_path(&canon)) {
        resp.headers_mut().insert(header::CONTENT_TYPE, val);
    }
    resp
}

async fn github_issues(
    State(state): State<AppState>,
    Query(q): Query<GithubIssuesQuery>,
) -> Json<Value> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "issue",
        "list",
        "--json",
        "number,title,url,state,author,createdAt",
        "--limit",
        "15",
    ]);
    if let Some(repo) = q.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.args(["--repo", repo]);
    } else if let Some(cwd) = q.cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let dir = md_core::config::expand_tilde(cwd);
        if dir.is_dir() {
            cmd.current_dir(dir);
        }
    } else if let Some(home) = state.harness_home() {
        cmd.current_dir(home);
    }
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            let issues = serde_json::from_slice::<Value>(&out.stdout).unwrap_or_else(|_| json!([]));
            Json(json!({ "ok": true, "issues": issues }))
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Json(json!({
                "ok": false,
                "issues": [],
                "error": if err.is_empty() { "gh issue list failed".to_string() } else { err }
            }))
        }
        Err(e) => Json(json!({ "ok": false, "issues": [], "error": e.to_string() })),
    }
}

pub fn fs_git_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/ide/state", get(ide_state))
        .route("/api/v1/ide/dirty", post(ide_dirty))
        .route("/api/v1/ide/open", post(ide_open))
        .route("/api/v1/fs/list", post(fs_list))
        .route("/api/v1/fs/read", post(fs_read))
        .route("/api/v1/fs/read-binary", post(fs_read_binary))
        .route("/api/v1/fs/write", post(fs_write))
        .route("/api/v1/fs/search", post(fs_search))
        .route("/api/v1/fs/mkdir", post(fs_mkdir))
        .route("/api/v1/fs/create", post(fs_create))
        .route("/api/v1/fs/rename", post(fs_rename))
        .route("/api/v1/fs/trash", post(fs_trash))
        .route("/api/v1/fs/stat-abs", post(fs_stat_abs))
        .route("/api/v1/workspaces/list", get(workspaces_list))
        .route("/api/v1/workspaces/browse", get(workspaces_browse))
        .route("/api/v1/workspaces/ensure", post(workspaces_ensure))
        .route("/api/v1/projects/list", get(projects_list))
        .route("/api/v1/fs/folder-state", post(fs_folder_state))
        .route(
            "/api/v1/fs/upload",
            post(fs_upload).layer(DefaultBodyLimit::max(36 * 1024 * 1024)),
        )
        .route("/api/v1/fs/raw", get(fs_raw))
        .route("/api/v1/tools/status", get(tools_status))
        .route("/api/v1/github/issues", get(github_issues))
        .route("/api/v1/git/is-repo", get(git_is_repo))
        .route("/api/v1/git/main-repo", get(git_main_repo))
        .route("/api/v1/git/branch", get(git_branch))
        .route("/api/v1/git/status", get(git_status))
        .route("/api/v1/git/log", get(git_log))
        .route("/api/v1/git/log-graph", get(git_log_graph))
        .route("/api/v1/git/commit-files", get(git_commit_files))
        .route("/api/v1/git/show-file", get(git_show_file))
        .route("/api/v1/git/compare-refs", get(git_compare_refs))
        .route("/api/v1/git/branches", get(git_branches))
        .route("/api/v1/git/ahead-behind", get(git_ahead_behind))
        .route("/api/v1/git/diff", get(git_diff))
        .route("/api/v1/git/worktrees", get(git_worktrees))
        .route("/api/v1/git/checkout", post(git_checkout))
        .route("/api/v1/worktrees/list", get(worktrees_list))
        .route("/api/v1/worktrees/size", post(worktrees_size))
        .route("/api/v1/worktrees/work", post(worktrees_work))
        .route("/api/v1/worktrees/remove", post(worktrees_remove))
}
