use crate::state::{AppState, IdeState};
use axum::{
    Json, Router,
    extract::{Query, State},
    response::Html,
    routing::{get, post},
};
use base64::Engine;
use md_core::repo_fs::{
    SearchOptions, create_file_in, list_dir, make_dir_in, read_file_binary, read_file_text,
    rename_in, search_in_root, trash_in, write_file_text,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

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

async fn ide_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en" data-cth-theme="dark" data-cth-skin="professional">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Munder Difflin IDE (Web)</title>
  <style>
    html, body { margin: 0; height: 100dvh; background: #17171B; color: #DEDBD6; font-family: Inter, system-ui, sans-serif; font-size: 13px; display: grid; place-items: center; }
  </style>
</head>
<body>
  <div id="ide-root">Web IDE Route (/ide) — BroadcastChannel + WS Buffer Sync Ready</div>
</body>
</html>"#,
    )
}

async fn ide_state(State(state): State<AppState>) -> Json<IdeState> {
    Json(state.ide.lock().unwrap().clone())
}

async fn ide_dirty(State(state): State<AppState>, Json(req): Json<IdeDirtyReq>) -> Json<Value> {
    {
        let mut ide = state.ide.lock().unwrap();
        ide.unsaved_count = req.unsaved;
        if !req.open_files.is_empty() {
            ide.open_files = req.open_files.clone();
        }
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

pub fn fs_git_router() -> Router<AppState> {
    Router::new()
        .route("/ide", get(ide_page))
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
        .route("/api/v1/git/is-repo", get(git_is_repo))
        .route("/api/v1/git/main-repo", get(git_main_repo))
        .route("/api/v1/git/branch", get(git_branch))
        .route("/api/v1/git/status", get(git_status))
        .route("/api/v1/git/log", get(git_log))
        .route("/api/v1/git/branches", get(git_branches))
        .route("/api/v1/git/ahead-behind", get(git_ahead_behind))
        .route("/api/v1/git/diff", get(git_diff))
        .route("/api/v1/git/worktrees", get(git_worktrees))
        .route("/api/v1/git/checkout", post(git_checkout))
        .route("/api/v1/worktrees/list", get(worktrees_list))
        .route("/api/v1/worktrees/size", post(worktrees_size))
}
