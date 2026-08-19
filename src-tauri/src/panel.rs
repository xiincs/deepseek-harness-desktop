//! Workspace file tree and git status — powers the native side panel that
//! sits beside the embedded harness `<iframe>` in `ui/`. Independent of dsh:
//! reads the workspace directory directly and shells out to the user's own
//! `git`, the same pattern `server.rs` already uses for `node`/`npm`.
//!
//! There is no dsh-side HTTP API for either of these (confirmed against the
//! upstream harness's own `/api` gateway, which exposes no filesystem or git
//! surface) — the upstream project's own design notes place an in-app file
//! preview "in the desktop shell's own design, not [the harness's]", so this
//! module owns both independently rather than proxying anything dsh-side.
//!
//! It does read one piece of dsh's own state directly off disk, though: see
//! `active_workspace_dir` below for why `DSH_DESKTOP_CWD` alone isn't enough
//! to know what workspace the panel should show.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use serde::Serialize;
use tauri_plugin_opener::OpenerExt;

/// Directory names never shown in the tree, regardless of workspace — `.git`
/// is never useful to browse here, and `node_modules`/`target` are the two
/// dependency/build directories common enough across arbitrary user
/// workspaces to be worth a hardcoded skip rather than requiring the user to
/// collapse them by hand every time. Deliberately not gitignore-aware (real
/// pattern matching, negation, and nested `.gitignore` files are a
/// meaningfully bigger scope than this first slice) — a workspace with other
/// large ignored directories still renders them today.
const IGNORED_DIR_NAMES: &[&str] = &[".git", "node_modules", "target"];
/// Caps a pathological workspace (a huge monorepo, or one of the ignored
/// names above not applying) from blocking the UI thread on a multi-second
/// directory walk. Higher than it originally was now that list_workspace_tree
/// spends this budget breadth-first (every already-discovered directory gets
/// a fair turn before the budget starts going toward anyone's
/// grandchildren) — a low cap mattered a lot more when one big subtree could
/// silently absorb the whole thing depth-first.
const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 12;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    pub name: String,
    /// Relative to the workspace root, `/`-separated regardless of platform
    /// (matches `GitEntry::path` below, so the client can join tree entries
    /// against git status by exact string equality).
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeEntry>>,
}

/// One directory still waiting for its own immediate children to be listed
/// — `indices` is the path down through `list_workspace_tree`'s `out` (and
/// each ancestor's own `children`) to reach that directory's `TreeEntry`.
struct PendingDir {
    indices: Vec<usize>,
    full_path: PathBuf,
    depth: usize,
}

pub fn list_workspace_tree(root: &Path) -> Vec<TreeEntry> {
    let mut budget = MAX_ENTRIES;
    list_workspace_tree_with_budget(root, &mut budget)
}

/// Breadth-first, not depth-first — this is the second attempt at this
/// function, and the difference from the first matters. The first version
/// recursed into each subdirectory immediately (depth-first): a large/deep
/// early branch could exhaust the *entire shared budget* before its own
/// unrelated siblings were even listed. That was fixed by listing a
/// directory's own entries before recursing into any of them (see
/// list_dir_shallow) — but that fix only made each *individual* directory
/// fair against its own children. The same starvation pattern still
/// recurred one level deeper: a huge subdirectory could still exhaust the
/// budget before a *sibling* two levels down ever got its own turn, at any
/// depth, not just the root. Reported twice now (root-level, then again for
/// folders one level in) — breadth-first actually closes this: every
/// directory at depth N gets its own immediate children listed before *any*
/// directory at depth N+1 does, so the budget can only ever run out
/// "fairly", after giving every already-discovered directory an equal turn
/// first. Takes `budget` as a parameter (list_workspace_tree above supplies
/// MAX_ENTRIES) so a test can exhaust a tiny budget without needing to
/// create thousands of real files.
fn list_workspace_tree_with_budget(root: &Path, budget: &mut usize) -> Vec<TreeEntry> {
    let mut out = list_dir_shallow(root, root, budget);

    let mut queue: VecDeque<PendingDir> = VecDeque::new();
    for (i, entry) in out.iter().enumerate() {
        if entry.is_dir {
            queue.push_back(PendingDir { indices: vec![i], full_path: root.join(&entry.name), depth: 1 });
        }
    }
    while let Some(pending) = queue.pop_front() {
        if pending.depth > MAX_DEPTH || *budget == 0 {
            continue;
        }
        let children = list_dir_shallow(root, &pending.full_path, budget);
        for (i, child) in children.iter().enumerate() {
            if child.is_dir {
                let mut indices = pending.indices.clone();
                indices.push(i);
                queue.push_back(PendingDir { indices, full_path: pending.full_path.join(&child.name), depth: pending.depth + 1 });
            }
        }
        node_at_mut(&mut out, &pending.indices).children = Some(children);
    }
    out
}

/// Walks `indices` down through `entries` and each ancestor's own
/// `children` to reach one specific `TreeEntry`. Every index path this is
/// ever called with comes from a `PendingDir` the BFS loop above enqueued
/// while processing that entry's *parent* — by the time this entry's own
/// turn comes up (queue order preserves discovery order), the parent's
/// `children` has already been assigned `Some(...)` in an earlier iteration
/// of that same loop, which is what the `expect` below is relying on.
fn node_at_mut<'a>(entries: &'a mut [TreeEntry], indices: &[usize]) -> &'a mut TreeEntry {
    let (&first, rest) = indices.split_first().expect("indices is never empty");
    let entry = &mut entries[first];
    if rest.is_empty() {
        entry
    } else {
        node_at_mut(entry.children.as_mut().expect("parent's own children were already assigned before this child was enqueued"), rest)
    }
}

/// Lists just `dir`'s own immediate entries — no recursion, unlike the
/// function this replaced. Every directory in the result starts with
/// `children: Some(vec![])`, a placeholder `list_workspace_tree`'s BFS loop
/// either fills in later or leaves as-is (for whichever directories the
/// shared budget didn't reach) — same "still renders as an expandable, if
/// empty-looking, folder rather than silently looking like a leaf" reason
/// the very first version of this budget scheme already established.
fn list_dir_shallow(root: &Path, dir: &Path, budget: &mut usize) -> Vec<TreeEntry> {
    if *budget == 0 {
        return Vec::new();
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = read_dir.filter_map(Result::ok).collect();
    // Directories first, then alphabetical within each group — stable,
    // predictable order for a tree UI (matches how most file browsers sort).
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir.cmp(&a_is_dir).then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut out = Vec::new();
    for entry in entries {
        if *budget == 0 {
            break;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = file_type.is_dir();
        if is_dir && IGNORED_DIR_NAMES.contains(&name.as_str()) {
            continue;
        }
        let full_path = entry.path();
        let Ok(rel) = full_path.strip_prefix(root) else { continue };
        let path = rel.to_string_lossy().replace('\\', "/");
        *budget -= 1;
        out.push(TreeEntry { name, path, is_dir, children: is_dir.then(Vec::new) });
    }
    out
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum GitStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
}

#[derive(Serialize, Clone)]
pub struct GitEntry {
    pub path: String,
    pub status: GitStatus,
}

// ── active workspace ─────────────────────────────────────────────────────
//
// `DSH_DESKTOP_CWD` (`server::workspace_dir`) is only the directory the
// shell happened to be launched with — a Windows Explorer "open with" arg,
// or an env var. The harness has its own, separate, in-page "workspace"
// concept (Settings → Choose workspace) that a user can register many of
// and switch between freely, entirely inside the iframe, with no signal
// reaching this shell (that's the whole point of the zero-IPC boundary).
// Observed directly: those two can disagree — the shell panel kept showing
// its launch directory while the harness UI had a completely different
// workspace open.
//
// There is no live "the user is looking at session X right now" signal to
// chase, on either side of that boundary — confirmed by direct network
// inspection of a running instance (`read_network_requests` in the Browser
// tab, not a guess): the harness loads every session's and workspace's full
// metadata once at boot, then switching between *existing* sessions or
// expanding a workspace group in the sidebar fires zero further HTTP
// requests — it's pure client-side state. Corroborated from two more
// angles: no client-side URL routing (`pushState`/`useNavigate`/router
// exports all zero-hit across `packages/client` in
// deepseek-ai/deepseek-harness) and no existing iframe↔parent postMessage
// channel (the only "postMessage" hits in that repo are Web Worker
// messaging and a native dialog IPC helper, unrelated to this). So "most
// recently sent a message" (`session.list`'s `updatedAt`, below) is the
// ceiling of precision available without either breaking the iframe's
// origin isolation or the harness itself shipping a new integration hook —
// not a shortcut taken here.
//
// Two sources, tried in order:
//
// 1. `POST {harness url}/api/session.list` — the canonical, always-current
//    source, confirmed live: dsh's own internal unary RPC-over-HTTP wire
//    format (`rpc_call` below), cross-checked against
//    `packages/host/apiproxy/src/{fetch/handler.ts,api/sessions.schema.ts}`
//    in deepseek-ai/deepseek-harness. Only reachable once the server is up.
// 2. `<dsh home>/storages/workspace.json` — a plain JSON file dsh persists
//    itself, covering the window before the server is reachable. Its
//    `global.workspaceIds[0]` was verified by direct observation to match
//    the on-screen active workspace at one point in time, but — unlike
//    `session.list` — was later observed to lag ordinary session activity
//    (its `updatedAt` sat minutes behind `session_projcache.json`'s during
//    continued use), so it's kept only as the fallback for when there's no
//    live server to ask, not trusted as the primary signal.
//
// Both are undocumented, internal-shaped payloads in a product whose own
// README states "developer preview... THERE WILL BE COMPATIBILITY-BREAKING
// CHANGES" — not a stable contract either way. Every step is `Option`-
// chained so any mismatch (server unreachable, missing file, mid-write
// partial read, renamed field, restructured schema) falls through cleanly,
// and every caller falls back to `workspace_dir` — this must never be a
// hard requirement.

/// Extracts the most-recently-updated session's `cwd` from a `session.list`
/// response's `items` array. Sessions without a `cwd` (e.g. subagent-origin
/// ones, per the schema's `cwd?: string`) are skipped rather than winning a
/// comparison against an absent timestamp.
fn latest_session_cwd(items: &[serde_json::Value]) -> Option<PathBuf> {
    items
        .iter()
        .filter_map(|item| {
            let updated_at = item.get("updatedAt")?.as_i64()?;
            let cwd = item.get("cwd")?.as_str()?;
            Some((updated_at, cwd))
        })
        .max_by_key(|(updated_at, _)| *updated_at)
        .map(|(_, cwd)| PathBuf::from(cwd))
}

/// dsh's internal unary RPC-over-HTTP wire format: POST a
/// `{type: "client-request", rpcId, method, payload: {}}` envelope to
/// `{base_url}/api/<method>`, expect back `{type: "server-response", rpcId,
/// result: {ok: true, value} | {ok: false, error}}`. `rpcId` doesn't need
/// real uniqueness here: exactly one request is ever in flight on this
/// connection, nothing correlates concurrent replies against it.
fn rpc_call(base_url: &str, method: &str) -> Option<serde_json::Value> {
    let url = format!("{base_url}/api/{method}");
    let body = serde_json::json!({
        "type": "client-request",
        "rpcId": "dsh-desktop-panel",
        "method": method,
        "payload": {},
    });
    let response: serde_json::Value = ureq::post(&url)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .build()
        .header("Content-Type", "application/json")
        .send_json(&body)
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;
    let result = response.get("result")?;
    if result.get("ok")?.as_bool()? {
        result.get("value").cloned()
    } else {
        None
    }
}

/// Core file-parsing logic, independent of `AppHandle` so it's directly
/// testable against a synthetic (or real, captured) `workspace.json` — see
/// `active_workspace_dir` for the entry point that supplies dsh's actual
/// storage directory.
fn parse_active_workspace(storages_dir: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(storages_dir.join("workspace.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let first_id = json.get("global")?.get("workspaceIds")?.as_array()?.first()?.as_str()?;
    let path = json.get("tables")?.get("workspaces")?.get(first_id)?.get("path")?.as_str()?;
    Some(PathBuf::from(path))
}

/// The harness's own currently-active workspace, when resolvable — see the
/// module-level note above.
pub fn active_workspace_dir(app: &tauri::AppHandle, server: &crate::server::Shared) -> Option<PathBuf> {
    if let Some(url) = crate::server::running_url(server) {
        if let Some(items) = rpc_call(&url, "session.list").and_then(|v| v.get("items")?.as_array().cloned()) {
            if let Some(cwd) = latest_session_cwd(&items) {
                return Some(cwd);
            }
        }
    }
    parse_active_workspace(&crate::server::dsh_home_dir(app).join("storages"))
}

/// What the panel should actually show: the harness's active workspace when
/// resolvable, else the directory this shell was launched with/spawns the
/// server in.
pub fn effective_workspace_dir(app: &tauri::AppHandle, server: &crate::server::Shared) -> PathBuf {
    active_workspace_dir(app, server).unwrap_or_else(|| crate::server::workspace_dir(app))
}

// ── known workspaces (manual picker) ─────────────────────────────────────
//
// The auto-inference above has a real, principled ceiling (see the note
// above it): nothing observes a pure "switched which existing session I'm
// looking at" action, only "sent a message in it". For a user who wants the
// panel pinned to a specific project regardless of that, the client
// (`ui/app.js`) offers a manual picker populated from this list and passes
// its choice back to `get_workspace_tree`/`get_git_status` as an explicit
// override — see those commands in `lib.rs`. This function only supplies
// the *options*; which one is currently selected is UI state the client
// owns and persists itself, not something Rust tracks.

#[derive(Serialize, Clone)]
pub struct WorkspaceOption {
    pub path: String,
    pub title: String,
}

fn parse_workspace_option(item: &serde_json::Value) -> Option<WorkspaceOption> {
    let path = item.get("path")?.as_str()?.to_string();
    let title = item.get("title")?.as_str()?.to_string();
    Some(WorkspaceOption { path, title })
}

/// Every workspace dsh currently knows about, `session.list`-API first
/// (`workspace.list`'s `items` are already in the server's own order — the
/// live source `active_workspace_dir` also prefers), falling back to the
/// persisted file when the server isn't reachable.
pub fn known_workspaces(app: &tauri::AppHandle, server: &crate::server::Shared) -> Vec<WorkspaceOption> {
    if let Some(url) = crate::server::running_url(server) {
        if let Some(items) = rpc_call(&url, "workspace.list").and_then(|v| v.get("items")?.as_array().cloned()) {
            let options: Vec<WorkspaceOption> = items.iter().filter_map(parse_workspace_option).collect();
            if !options.is_empty() {
                return options;
            }
        }
    }
    file_workspace_options(&crate::server::dsh_home_dir(app).join("storages"))
}

/// Core file-parsing logic, independent of `AppHandle` — mirrors
/// `parse_active_workspace`'s fallback but returns every entry (in
/// `workspaceIds` order) rather than just the first.
fn file_workspace_options(storages_dir: &Path) -> Vec<WorkspaceOption> {
    let Ok(text) = fs::read_to_string(storages_dir.join("workspace.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(ids) = json.get("global").and_then(|g| g.get("workspaceIds")).and_then(|w| w.as_array()) else {
        return Vec::new();
    };
    let Some(workspaces) = json.get("tables").and_then(|t| t.get("workspaces")) else {
        return Vec::new();
    };
    ids.iter()
        .filter_map(|id| id.as_str())
        .filter_map(|id| workspaces.get(id))
        .filter_map(parse_workspace_option)
        .collect()
}

/// Runs `git status` in `cwd` and returns per-path status. Returns an empty
/// list — not an error — when `cwd` isn't a git repository, or `git` isn't
/// on PATH: the file tree above must stay usable either way, git status
/// coloring is a bonus overlay, not a precondition.
///
/// `--no-renames`: porcelain `-z` rename records put the *new* path in one
/// NUL-separated field and the *old* path in the next, and getting that
/// order backwards would silently mislabel a renamed file under its old
/// name. Rather than trust an unverified recollection of that field order,
/// renames are reported as a plain delete + add pair instead — the accurate
/// simplification given this slice's own testing already exercises the
/// modify/add/delete/untracked cases and no more.
pub fn git_status(cwd: &Path) -> Vec<GitEntry> {
    let mut cmd = Command::new("git");
    cmd.args(["status", "--porcelain=v1", "--untracked-files=all", "--no-renames", "-z"]);
    cmd.current_dir(crate::server::plain_win_path(cwd));
    crate::server::hide_console(&mut cmd);
    let Ok(output) = cmd.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split('\0').filter(|record| record.len() > 3).filter_map(parse_porcelain_record).collect()
}

fn parse_porcelain_record(record: &str) -> Option<GitEntry> {
    let mut chars = record.chars();
    let x = chars.next()?;
    let y = chars.next()?;
    let path = record.get(3..)?.replace('\\', "/");
    let status = if x == '?' && y == '?' {
        GitStatus::Untracked
    } else if x == 'A' {
        GitStatus::Added
    } else if x == 'D' || y == 'D' {
        GitStatus::Deleted
    } else {
        GitStatus::Modified
    };
    Some(GitEntry { path, status })
}

// ── file preview / edit ──────────────────────────────────────────────────
//
// Diffing is no longer computed here — `ui/app.js` hands both `current` and
// `original` to CodeMirror's own `unifiedMergeView`, which diffs and renders
// them client-side inside one always-editable pane (replacing an earlier,
// separate read-only hand-rolled diff-hunk view; see the git history for
// that version if it's ever useful again). This module's job narrows to
// fetching the two documents each side needs.

/// Generous ceiling on what this reads into memory — large enough for real
/// source files, small enough that an accidentally-selected generated/data
/// file doesn't stall the panel.
const MAX_PREVIEW_BYTES: u64 = 2_000_000;

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileContent {
    Text { content: String },
    /// Rendered as an <img>, not CodeMirror — see image_mime_type for which
    /// extensions take this path.
    Image { mime: String, base64: String },
    Binary,
    TooLarge { bytes: u64 },
    Error { message: String },
}

#[derive(Serialize, Clone)]
pub struct EditablePreview {
    /// Current on-disk content — `None` only for a file git reports
    /// `Deleted` (there is nothing left to read from disk).
    pub current: Option<FileContent>,
    /// Last-committed (`HEAD`) content, present only when there's a prior
    /// version worth comparing against (`Modified`/`Deleted`) — `None` for
    /// an untracked or unchanged file, where nothing has diverged from HEAD
    /// in a way this panel needs to show.
    pub original: Option<FileContent>,
}

/// `rel_path`: workspace-relative, `/`-separated — the same shape
/// `TreeEntry.path`/`GitEntry.path` already use, so the client passes
/// straight through whichever tree row it clicked.
pub fn editable_preview(root: &Path, rel_path: &str) -> EditablePreview {
    let status = git_status(root).into_iter().find(|e| e.path == rel_path).map(|e| e.status);
    let current = if matches!(status, Some(GitStatus::Deleted)) {
        None
    } else {
        Some(text_preview(root, rel_path))
    };
    let original = match status {
        Some(GitStatus::Modified) | Some(GitStatus::Deleted) => git_show_head(root, rel_path),
        _ => None,
    };
    EditablePreview { current, original }
}

/// Plain file content read straight off disk — also the entry point
/// `save_file`'s round-trip verification and any other consumer that just
/// wants "what does this file currently contain" reaches for.
pub fn text_preview(root: &Path, rel_path: &str) -> FileContent {
    let lang = crate::i18n::detect();
    let full_path = root.join(rel_path);
    let Ok(metadata) = fs::metadata(&full_path) else {
        return FileContent::Error {
            message: crate::i18n::tr(lang, "文件不存在", "File does not exist").to_string(),
        };
    };
    if metadata.len() > MAX_PREVIEW_BYTES {
        return FileContent::TooLarge { bytes: metadata.len() };
    }
    let Ok(bytes) = fs::read(&full_path) else {
        return FileContent::Error {
            message: crate::i18n::tr(lang, "无法读取文件", "Unable to read file").to_string(),
        };
    };
    if let Some(mime) = image_mime_type(rel_path) {
        return FileContent::Image { mime: mime.to_string(), base64: base64::engine::general_purpose::STANDARD.encode(&bytes) };
    }
    // A NUL byte anywhere is a cheap, reliable-enough binary heuristic —
    // the same one git itself uses to decide whether a file diffs as text.
    if bytes.contains(&0) {
        return FileContent::Binary;
    }
    match String::from_utf8(bytes) {
        Ok(content) => FileContent::Text { content },
        Err(_) => FileContent::Binary,
    }
}

/// Extension → MIME type for the raster formats this panel renders as an
/// `<img>` instead of the "binary, no preview" fallback. Deliberately
/// excludes `.svg`: unlike these, an SVG file is valid UTF-8 text and
/// already opens as an ordinary editable file today (it fails both the
/// NUL-byte and utf8 checks below the same way any other text file would) —
/// routing it through this path instead would trade away editing for a
/// preview nothing asked to remove.
fn image_mime_type(rel_path: &str) -> Option<&'static str> {
    let ext = rel_path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// The file's content as of `HEAD` via `git show`, independent of the
/// working tree — `None` when the path didn't exist at `HEAD` (e.g. a
/// brand-new repo with no commits yet) rather than an error, since that's
/// an ordinary, expected outcome this function's callers already treat as
/// "nothing to compare against".
fn git_show_head(root: &Path, rel_path: &str) -> Option<FileContent> {
    let mut cmd = Command::new("git");
    cmd.args(["show", &format!("HEAD:{rel_path}")]);
    cmd.current_dir(crate::server::plain_win_path(root));
    crate::server::hide_console(&mut cmd);
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    if output.stdout.len() as u64 > MAX_PREVIEW_BYTES {
        return Some(FileContent::TooLarge { bytes: output.stdout.len() as u64 });
    }
    if output.stdout.contains(&0) {
        return Some(FileContent::Binary);
    }
    match String::from_utf8(output.stdout) {
        Ok(content) => Some(FileContent::Text { content }),
        Err(_) => Some(FileContent::Binary),
    }
}

/// Resolves `rel_path` under `root` and verifies the result is actually
/// still *inside* `root` before any write touches disk. `rel_path` only
/// ever originates from this panel's own tree listing today (never
/// arbitrary user-typed input), so a `..`-escaping value shouldn't be
/// reachable in practice — this check exists anyway because a write is a
/// meaningfully higher-consequence operation than the read paths above,
/// where the same class of bug would only misdirect a preview rather than
/// overwrite a file outside the workspace.
fn resolve_within_root(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let lang = crate::i18n::detect();
    let candidate = root.join(rel_path);
    let root_canon = root.canonicalize().map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Failed to resolve workspace path: {e}")
        } else {
            format!("无法解析工作区路径: {e}")
        }
    })?;
    let candidate_canon = candidate.canonicalize().map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Failed to resolve file path: {e}")
        } else {
            format!("无法解析文件路径: {e}")
        }
    })?;
    if !candidate_canon.starts_with(&root_canon) {
        return Err(crate::i18n::tr(lang, "路径超出工作区范围", "Path is outside the workspace").to_string());
    }
    Ok(candidate_canon)
}

/// Overwrites an existing file's content. Refuses to create a new file
/// (`resolve_within_root`'s `canonicalize()` requires the target to already
/// exist) — this is an edit-and-save path for a file the tree already
/// listed, not a general file-creation API.
pub fn save_file(root: &Path, rel_path: &str, content: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    let full_path = resolve_within_root(root, rel_path)?;
    fs::write(&full_path, content).map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Save failed: {e}")
        } else {
            format!("保存失败: {e}")
        }
    })
}

// ── file/folder operations (tree context menu) ──────────────────────────
//
// Unlike every rel_path above (which only ever originates from this panel's
// own tree listing — see resolve_within_root's doc comment), the *names*
// here come straight from a text input the user typed (New File/Folder's
// name, Rename's new name) — validate_entry_name is this section's own,
// stricter gate for that, checked before any of these ever touch disk.
// resolve_within_root itself is still what validates every *existing* path
// (a parent to create inside, an entry to rename/move/delete) stays inside
// the workspace, same as save_file above.

/// A single path segment, never something that could traverse elsewhere —
/// rejects anything empty, `.`/`..`, containing a path separator, or (on
/// Windows) one of the characters NTFS itself won't allow in a name.
fn validate_entry_name(name: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    let windows_reserved = |c: char| "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20;
    let invalid = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(windows_reserved);
    if invalid {
        return Err(crate::i18n::tr(lang, "文件名无效", "Invalid file name").to_string());
    }
    Ok(())
}

fn err_already_exists() -> String {
    let lang = crate::i18n::detect();
    crate::i18n::tr(lang, "已存在同名文件或文件夹", "A file or folder with this name already exists").to_string()
}

/// `parent_rel_path`: workspace-relative, from the tree (`""` for the
/// workspace root itself — `root.join("")` is `root` unchanged, so that
/// resolves correctly too). `name`: user-typed, the new file's own name
/// (not a path).
pub fn create_file(root: &Path, parent_rel_path: &str, name: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    validate_entry_name(name)?;
    let parent = resolve_within_root(root, parent_rel_path)?;
    let target = parent.join(name);
    if target.exists() {
        return Err(err_already_exists());
    }
    fs::write(&target, "").map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Failed to create file: {e}")
        } else {
            format!("创建文件失败: {e}")
        }
    })
}

pub fn create_dir(root: &Path, parent_rel_path: &str, name: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    validate_entry_name(name)?;
    let parent = resolve_within_root(root, parent_rel_path)?;
    let target = parent.join(name);
    if target.exists() {
        return Err(err_already_exists());
    }
    fs::create_dir(&target).map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Failed to create folder: {e}")
        } else {
            format!("创建文件夹失败: {e}")
        }
    })
}

/// `rel_path`: the entry being renamed, from the tree. `new_name`:
/// user-typed — just the new name, not a path, so this can't be used to
/// move an entry into a different directory (see move_entry for that).
pub fn rename_entry(root: &Path, rel_path: &str, new_name: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    validate_entry_name(new_name)?;
    let source = resolve_within_root(root, rel_path)?;
    let parent = source.parent().ok_or_else(|| crate::i18n::tr(lang, "无法重命名工作区根目录", "Cannot rename the workspace root").to_string())?;
    let target = parent.join(new_name);
    if target.exists() {
        return Err(err_already_exists());
    }
    fs::rename(&source, &target).map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Rename failed: {e}")
        } else {
            format!("重命名失败: {e}")
        }
    })
}

/// Drag-and-drop move: `from_rel_path` (the dragged entry) into
/// `to_parent_rel_path` (the folder dropped onto, `""` for the workspace
/// root), keeping the same base name — both sides are tree-derived paths,
/// not user-typed, so this doesn't go through validate_entry_name at all.
pub fn move_entry(root: &Path, from_rel_path: &str, to_parent_rel_path: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    let source = resolve_within_root(root, from_rel_path)?;
    let target_parent = resolve_within_root(root, to_parent_rel_path)?;
    let name = source
        .file_name()
        .ok_or_else(|| crate::i18n::tr(lang, "无法移动工作区根目录", "Cannot move the workspace root").to_string())?;
    // fs::rename onto a directory's own descendant is nonsensical (and
    // platform-inconsistent about how it fails) — caught explicitly here
    // for a clear message instead of surfacing whatever raw OS error that
    // produces. starts_with also covers "dropped onto itself" (source ==
    // target_parent is a prefix of itself).
    if target_parent.starts_with(&source) {
        return Err(crate::i18n::tr(lang, "不能将文件夹移动到自身或其子目录中", "Cannot move a folder into itself or one of its own subfolders").to_string());
    }
    let target = target_parent.join(name);
    if target.exists() {
        return Err(err_already_exists());
    }
    fs::rename(&source, &target).map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Move failed: {e}")
        } else {
            format!("移动失败: {e}")
        }
    })
}

/// To the OS recycle bin/trash, not a permanent fs::remove_* — see this
/// section's own module comment and the `trash` dependency's doc comment in
/// Cargo.toml for why. Works for both files and directories.
pub fn delete_entry(root: &Path, rel_path: &str) -> Result<(), String> {
    let lang = crate::i18n::detect();
    let target = resolve_within_root(root, rel_path)?;
    trash::delete(&target).map_err(|e| {
        if lang == crate::i18n::Lang::En {
            format!("Delete failed: {e}")
        } else {
            format!("删除失败: {e}")
        }
    })
}

/// Opens the OS's own file manager with `rel_path` selected — the same
/// `reveal_item_in_dir` the app menu's "打开数据目录"/open_data_dir already
/// uses in lib.rs, just pointed at a workspace entry instead of the dsh
/// home directory.
pub fn reveal_in_file_manager(app: &tauri::AppHandle, root: &Path, rel_path: &str) -> Result<(), String> {
    let target = resolve_within_root(root, rel_path)?;
    app.opener().reveal_item_in_dir(&target).map_err(|e| e.to_string())
}

/// The tree's "Copy Path" context menu item: `rel_path` resolved to a real,
/// OS-native absolute path string (backslash-separated on Windows) — the
/// client only ever has the `/`-separated workspace-relative form
/// (TreeEntry.path), and string-concatenating that onto a workspace root
/// itself in JS would need to reimplement platform-correct path joining
/// there for no reason when Path::join already does it correctly here.
///
/// Strips Windows' `\\?\` verbatim-path prefix if present: `canonicalize()`
/// (inside resolve_within_root) returns paths in that form on Windows —
/// correct and necessary for actual filesystem calls (bypasses MAX_PATH),
/// but not something a user wants to see after pasting "Copy Path" into
/// Explorer's address bar or a random other tool, several of which don't
/// handle it well.
pub fn absolute_path(root: &Path, rel_path: &str) -> Result<String, String> {
    let resolved = resolve_within_root(root, rel_path)?;
    let s = resolved.to_string_lossy();
    Ok(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git").current_dir(dir).args(args).status().expect("git must be on PATH to run this test");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-desktop-panel-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Trimmed from a real, captured `POST /api/session.list` response body
    // (read live off a running instance's Network tab; irrelevant
    // `projections`/`agentPreset`/etc fields dropped since this parser only
    // reads `updatedAt`/`cwd`) — three sessions across two workspaces, the
    // middle one most recently updated despite not being listed last.
    fn real_session_list_items() -> Vec<serde_json::Value> {
        serde_json::from_str(
            r#"[
              {"sessionId": "session-e5e407e6", "updatedAt": 1786774994999, "cwd": "E:\\Project202608\\ExampleProject001"},
              {"sessionId": "session-0f2b0cdf", "updatedAt": 1786775697190, "cwd": "E:\\Project202608\\ExampleProject001"},
              {"sessionId": "session-2390bcfc", "updatedAt": 1786733386399, "cwd": "E:\\Project202608\\HappyTime"}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn latest_session_cwd_picks_the_max_updated_at_not_list_order() {
        let items = real_session_list_items();
        assert_eq!(
            latest_session_cwd(&items),
            Some(PathBuf::from(r"E:\Project202608\ExampleProject001"))
        );
    }

    #[test]
    fn latest_session_cwd_skips_entries_without_a_cwd() {
        let items: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
              {"sessionId": "session-subagent", "updatedAt": 9999999999999},
              {"sessionId": "session-real", "updatedAt": 1, "cwd": "E:\\only\\real\\one"}
            ]"#,
        )
        .unwrap();
        assert_eq!(latest_session_cwd(&items), Some(PathBuf::from(r"E:\only\real\one")));
    }

    #[test]
    fn latest_session_cwd_of_empty_list_is_none() {
        assert_eq!(latest_session_cwd(&[]), None);
    }

    // Trimmed from a real, captured `~/.dsh/storages/workspace.json` (seven
    // registered workspaces; `sessionIds`/timestamps/session-scoped data
    // dropped since this parser never reads them) — real field names and
    // real nesting, not a hand-typed guess at the shape.
    const REAL_WORKSPACE_JSON: &str = r#"{
      "unit": { "name": "workspace", "version": 2 },
      "global": {
        "initialized": true,
        "workspaceIds": [
          "bf8d8e2c-1049-4f40-bac4-f6ffe614aa59",
          "13d72047-972b-42cf-98e3-1c3605728319",
          "1c98bf74-32e0-4ad5-b0c0-66906a4778a4"
        ],
        "archivedSessionIds": []
      },
      "tables": {
        "workspaces": {
          "1c98bf74-32e0-4ad5-b0c0-66906a4778a4": {
            "path": "E:\\Project202608\\dsh-desktop",
            "title": "dsh-desktop"
          },
          "13d72047-972b-42cf-98e3-1c3605728319": {
            "path": "E:\\Project202608\\HappyTime",
            "title": "HappyTime"
          },
          "bf8d8e2c-1049-4f40-bac4-f6ffe614aa59": {
            "path": "E:\\Project202608\\ExampleProject001",
            "title": "ExampleProject001"
          }
        }
      }
    }"#;

    #[test]
    fn parses_the_first_workspace_ids_entry_from_a_real_captured_file() {
        let dir = scratch_dir("workspace-json");
        fs::write(dir.join("workspace.json"), REAL_WORKSPACE_JSON).unwrap();

        assert_eq!(
            parse_active_workspace(&dir),
            Some(PathBuf::from(r"E:\Project202608\ExampleProject001"))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_workspace_options_returns_all_entries_in_workspace_ids_order() {
        let dir = scratch_dir("workspace-json-options");
        fs::write(dir.join("workspace.json"), REAL_WORKSPACE_JSON).unwrap();

        let options = file_workspace_options(&dir);
        let titles: Vec<_> = options.iter().map(|o| o.title.as_str()).collect();
        // Matches workspaceIds order in REAL_WORKSPACE_JSON, not tables.workspaces'
        // (arbitrary, UUID-keyed) object-key order.
        assert_eq!(titles, vec!["ExampleProject001", "HappyTime", "dsh-desktop"]);
        assert_eq!(options[0].path, r"E:\Project202608\ExampleProject001");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_workspace_options_on_missing_file_is_empty() {
        let dir = scratch_dir("workspace-json-options-missing");
        assert!(file_workspace_options(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_workspace_json_returns_none_not_an_error() {
        let dir = scratch_dir("workspace-json-missing");
        assert_eq!(parse_active_workspace(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_workspace_json_returns_none_not_a_panic() {
        let dir = scratch_dir("workspace-json-malformed");
        fs::write(dir.join("workspace.json"), "{ this is not valid json").unwrap();
        assert_eq!(parse_active_workspace(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_workspace_ids_returns_none() {
        let dir = scratch_dir("workspace-json-empty");
        fs::write(
            dir.join("workspace.json"),
            r#"{"global": {"workspaceIds": []}, "tables": {"workspaces": {}}}"#,
        )
        .unwrap();
        assert_eq!(parse_active_workspace(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_status_reports_modified_deleted_untracked_against_a_real_commit() {
        let dir = scratch_dir("status");
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "test"]);

        fs::write(dir.join("keep.txt"), "a").unwrap();
        fs::write(dir.join("gone.txt"), "b").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "init"]);

        fs::write(dir.join("keep.txt"), "a-changed").unwrap();
        fs::remove_file(dir.join("gone.txt")).unwrap();
        fs::write(dir.join("new.txt"), "c").unwrap();

        let entries = git_status(&dir);
        let find = |p: &str| entries.iter().find(|e| e.path == p).map(|e| e.status);

        assert_eq!(find("keep.txt"), Some(GitStatus::Modified));
        assert_eq!(find("gone.txt"), Some(GitStatus::Deleted));
        assert_eq!(find("new.txt"), Some(GitStatus::Untracked));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_status_on_a_non_git_directory_is_empty_not_an_error() {
        let dir = scratch_dir("nogit");
        assert!(git_status(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_skips_ignored_dir_names_and_sorts_directories_before_files() {
        let dir = scratch_dir("tree");
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("a.txt"), "").unwrap();
        fs::write(dir.join("src").join("main.rs"), "").unwrap();

        let tree = list_workspace_tree(&dir);
        let names: Vec<_> = tree.iter().map(|e| e.name.as_str()).collect();

        assert!(!names.contains(&"node_modules"));
        assert_eq!(names, vec!["src", "a.txt"]);
        assert_eq!(tree[0].path, "src");
        assert_eq!(tree[0].children.as_ref().unwrap()[0].path, "src/main.rs");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_lists_every_root_entry_even_when_an_earlier_ones_subtree_exhausts_the_budget() {
        // Regression test for a reported bug: a large/deep subtree under an
        // alphabetically-earlier root entry used to consume the *entire*
        // shared budget depth-first before its own root-level siblings were
        // even listed — not just before their children were fetched, they
        // never got a TreeEntry at all. Calls list_workspace_tree_with_budget
        // directly with a tiny budget instead of creating thousands of files
        // to exhaust the real MAX_ENTRIES.
        let dir = scratch_dir("tree-budget");
        fs::create_dir_all(dir.join("aaa")).unwrap();
        for i in 1..=5 {
            fs::write(dir.join("aaa").join(format!("f{i}.txt")), "").unwrap();
        }
        fs::create_dir_all(dir.join("bbb")).unwrap();
        fs::write(dir.join("bbb").join("g.txt"), "").unwrap();

        let mut budget = 3;
        let tree = list_workspace_tree_with_budget(&dir, &mut budget);
        let names: Vec<_> = tree.iter().map(|e| e.name.as_str()).collect();

        assert_eq!(names, vec!["aaa", "bbb"], "bbb must still be listed even though aaa's subtree used up the rest of the budget");
        assert!(!tree[0].children.as_ref().unwrap().is_empty(), "aaa should still get whatever budget remained after both root entries were listed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_lists_a_sibling_directorys_own_children_even_when_an_earlier_siblings_subtree_is_deep() {
        // Distinguishes true breadth-first from the first fix above, which
        // only made each *individual* directory's own listing fair against
        // its own children — not the whole tree fair against itself. aaa's
        // own subtree here goes three levels deep, deep enough that a
        // depth-first walk exhausts the whole budget completing it before
        // ever returning to list bbb/ccc's own, much shallower files —
        // reproducing the reported "only the first few folders show their
        // contents when expanded, later ones show nothing" one level below
        // the root, exactly where the first fix's root-level-only fairness
        // didn't reach.
        let dir = scratch_dir("tree-budget-deep-sibling");
        fs::create_dir_all(dir.join("parent").join("aaa").join("sub").join("subsub")).unwrap();
        fs::write(dir.join("parent").join("aaa").join("x1.txt"), "").unwrap();
        fs::write(dir.join("parent").join("aaa").join("sub").join("x2.txt"), "").unwrap();
        fs::write(dir.join("parent").join("aaa").join("sub").join("subsub").join("x3.txt"), "").unwrap();
        fs::create_dir_all(dir.join("parent").join("bbb")).unwrap();
        fs::write(dir.join("parent").join("bbb").join("g.txt"), "").unwrap();
        fs::create_dir_all(dir.join("parent").join("ccc")).unwrap();
        fs::write(dir.join("parent").join("ccc").join("h.txt"), "").unwrap();

        // root(1) + parent's own [aaa,bbb,ccc](3) + aaa's own [sub,x1.txt](2)
        // + sub's own [subsub,x2.txt](2) + subsub's own [x3.txt](1) = 9,
        // exactly enough for a depth-first walk to fully exhaust aaa's own
        // subtree before parent's loop ever reaches bbb.
        let mut budget = 9;
        let tree = list_workspace_tree_with_budget(&dir, &mut budget);

        let parent = tree.iter().find(|e| e.name == "parent").unwrap();
        let bbb = parent.children.as_ref().unwrap().iter().find(|e| e.name == "bbb").unwrap();
        let bbb_names: Vec<_> = bbb.children.as_ref().unwrap().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(bbb_names, vec!["g.txt"], "bbb's own file must be listed even though aaa's own subtree goes several levels deeper");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── file preview / edit ─────────────────────────────────────────────

    fn git_repo_with_one_commit(name: &str) -> std::path::PathBuf {
        let dir = scratch_dir(name);
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "test"]);
        dir
    }

    fn text_of(content: Option<FileContent>) -> String {
        match content {
            Some(FileContent::Text { content }) => content,
            other => panic!("expected Some(Text), got {other:?}"),
        }
    }

    #[test]
    fn editable_preview_of_a_modified_file_has_both_current_and_head_content() {
        let dir = git_repo_with_one_commit("editable-modified");
        fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "init"]);
        fs::write(dir.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();

        let preview = editable_preview(&dir, "a.txt");
        assert_eq!(text_of(preview.current), "one\nTWO\nthree\nfour\n");
        assert_eq!(text_of(preview.original), "one\ntwo\nthree\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn editable_preview_of_a_deleted_file_has_head_content_but_no_current() {
        let dir = git_repo_with_one_commit("editable-deleted");
        fs::write(dir.join("gone.txt"), "still in history\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "init"]);
        fs::remove_file(dir.join("gone.txt")).unwrap();

        let preview = editable_preview(&dir, "gone.txt");
        assert!(preview.current.is_none());
        assert_eq!(text_of(preview.original), "still in history\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn editable_preview_of_an_untracked_file_has_current_but_no_original() {
        let dir = git_repo_with_one_commit("editable-untracked");
        fs::write(dir.join("new.txt"), "brand new content\n").unwrap();

        let preview = editable_preview(&dir, "new.txt");
        assert_eq!(text_of(preview.current), "brand new content\n");
        assert!(preview.original.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn editable_preview_of_an_unchanged_tracked_file_has_no_original() {
        let dir = git_repo_with_one_commit("editable-unchanged");
        fs::write(dir.join("stable.txt"), "nothing to see here\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "init"]);

        let preview = editable_preview(&dir, "stable.txt");
        assert_eq!(text_of(preview.current), "nothing to see here\n");
        assert!(preview.original.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_show_head_of_a_path_absent_at_head_is_none_not_an_error() {
        let dir = git_repo_with_one_commit("show-head-absent");
        // No commits reference this path at all — a brand-new, wholly
        // untracked file, not merely one with local edits.
        assert!(git_show_head(&dir, "never-committed.txt").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_of_a_missing_path_is_an_error_not_a_panic() {
        let dir = scratch_dir("preview-missing");
        let preview = text_preview(&dir, "does-not-exist.txt");
        assert!(matches!(preview, FileContent::Error { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_of_binary_content_is_binary_not_text() {
        let dir = scratch_dir("preview-binary");
        fs::write(dir.join("blob.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
        let preview = text_preview(&dir, "blob.bin");
        assert!(matches!(preview, FileContent::Binary));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_of_a_png_is_image_not_binary() {
        let dir = scratch_dir("preview-image");
        let bytes = [0x89u8, 0x50, 0x4e, 0x47, 0, 1, 2, 3];
        fs::write(dir.join("pic.png"), bytes).unwrap();
        match text_preview(&dir, "pic.png") {
            FileContent::Image { mime, base64 } => {
                assert_eq!(mime, "image/png");
                assert_eq!(base64::engine::general_purpose::STANDARD.decode(base64).unwrap(), bytes);
            }
            other => panic!("expected Image, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_of_an_svg_is_still_text_not_image() {
        // .svg is deliberately excluded from image_mime_type — it's valid
        // UTF-8 XML and should stay editable, not become a read-only <img>.
        let dir = scratch_dir("preview-svg");
        fs::write(dir.join("icon.svg"), "<svg></svg>").unwrap();
        let preview = text_preview(&dir, "icon.svg");
        assert!(matches!(preview, FileContent::Text { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    // ── save ────────────────────────────────────────────────────────────

    #[test]
    fn save_file_overwrites_existing_content() {
        let dir = scratch_dir("save-ok");
        fs::write(dir.join("a.txt"), "old").unwrap();

        let result = save_file(&dir, "a.txt", "new content");
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "new content");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_file_writes_into_a_nested_path() {
        let dir = scratch_dir("save-nested");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("main.rs"), "old").unwrap();

        let result = save_file(&dir, "src/main.rs", "fn main() {}");
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(dir.join("src").join("main.rs")).unwrap(), "fn main() {}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_file_rejects_a_path_that_escapes_root() {
        let dir = scratch_dir("save-escape");
        let outside = scratch_dir("save-escape-outside-target");
        fs::write(outside.join("secret.txt"), "should not be touched").unwrap();

        // "../save-escape-outside-target-<pid>/secret.txt" — walks back out
        // of `dir` to a real, existing file elsewhere on disk. Both scratch
        // dirs share std::env::temp_dir() as a parent, so ".." reaches it.
        let escaping_rel_path = format!("../{}/secret.txt", outside.file_name().unwrap().to_string_lossy());

        let result = save_file(&dir, &escaping_rel_path, "overwritten");
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(outside.join("secret.txt")).unwrap(), "should not be touched");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn save_file_rejects_a_nonexistent_target() {
        let dir = scratch_dir("save-missing");
        let result = save_file(&dir, "does-not-exist.txt", "content");
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    // ── file/folder operations ────────────────────────────────────────────

    #[test]
    fn create_file_creates_an_empty_file_at_the_workspace_root() {
        let dir = scratch_dir("create-file-root");
        let result = create_file(&dir, "", "new.txt");
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_file_creates_inside_an_existing_subfolder() {
        let dir = scratch_dir("create-file-nested");
        fs::create_dir_all(dir.join("src")).unwrap();
        let result = create_file(&dir, "src", "main.rs");
        assert!(result.is_ok());
        assert!(dir.join("src").join("main.rs").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_file_rejects_a_name_that_already_exists() {
        let dir = scratch_dir("create-file-exists");
        fs::write(dir.join("a.txt"), "original").unwrap();
        let result = create_file(&dir, "", "a.txt");
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "original");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_file_rejects_a_name_containing_a_path_separator() {
        // The one check that matters most here: without it, a "new file
        // name" of "../escape.txt" would let create_file write outside
        // whatever parent_rel_path resolved to.
        let dir = scratch_dir("create-file-traversal");
        let result = create_file(&dir, "", "../escape.txt");
        assert!(result.is_err());
        assert!(!dir.parent().unwrap().join("escape.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_dir_creates_a_directory() {
        let dir = scratch_dir("create-dir");
        let result = create_dir(&dir, "", "newfolder");
        assert!(result.is_ok());
        assert!(dir.join("newfolder").is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_entry_renames_within_the_same_parent() {
        let dir = scratch_dir("rename-basic");
        fs::write(dir.join("old.txt"), "content").unwrap();
        let result = rename_entry(&dir, "old.txt", "new.txt");
        assert!(result.is_ok());
        assert!(!dir.join("old.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_entry_rejects_a_new_name_that_already_exists() {
        let dir = scratch_dir("rename-collision");
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("b.txt"), "b").unwrap();
        let result = rename_entry(&dir, "a.txt", "b.txt");
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "b");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_entry_moves_into_a_different_parent() {
        let dir = scratch_dir("move-basic");
        fs::create_dir_all(dir.join("dest")).unwrap();
        fs::write(dir.join("file.txt"), "content").unwrap();
        let result = move_entry(&dir, "file.txt", "dest");
        assert!(result.is_ok());
        assert!(!dir.join("file.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("dest").join("file.txt")).unwrap(), "content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_entry_rejects_moving_a_folder_into_its_own_subfolder() {
        let dir = scratch_dir("move-into-descendant");
        fs::create_dir_all(dir.join("parent").join("child")).unwrap();
        let result = move_entry(&dir, "parent", "parent/child");
        assert!(result.is_err());
        assert!(dir.join("parent").join("child").is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_entry_rejects_a_target_name_that_already_exists() {
        let dir = scratch_dir("move-collision");
        fs::create_dir_all(dir.join("dest")).unwrap();
        fs::write(dir.join("dest").join("file.txt"), "existing").unwrap();
        fs::write(dir.join("file.txt"), "incoming").unwrap();
        let result = move_entry(&dir, "file.txt", "dest");
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(dir.join("dest").join("file.txt")).unwrap(), "existing");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_entry_removes_the_file_from_its_original_location() {
        let dir = scratch_dir("delete-file");
        fs::write(dir.join("gone.txt"), "content").unwrap();
        let result = delete_entry(&dir, "gone.txt");
        assert!(result.is_ok(), "{result:?}");
        assert!(!dir.join("gone.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_entry_rejects_a_path_that_escapes_root() {
        let dir = scratch_dir("delete-escape");
        let outside = scratch_dir("delete-escape-outside-target");
        fs::write(outside.join("secret.txt"), "should not be touched").unwrap();
        let escaping_rel_path = format!("../{}/secret.txt", outside.file_name().unwrap().to_string_lossy());

        let result = delete_entry(&dir, &escaping_rel_path);
        assert!(result.is_err());
        assert!(outside.join("secret.txt").exists());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn absolute_path_resolves_a_workspace_relative_path() {
        let dir = scratch_dir("abs-path");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src").join("main.rs"), "").unwrap();
        let result = absolute_path(&dir, "src/main.rs").unwrap();
        let expected = dir.join("src").join("main.rs").canonicalize().unwrap();
        let expected_str = expected.to_string_lossy();
        let expected_stripped = expected_str.strip_prefix(r"\\?\").unwrap_or(&expected_str);
        assert_eq!(result, expected_stripped);
        assert!(!result.starts_with(r"\\?\"), "Copy Path shouldn't show Windows' verbatim-path prefix: {result}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn absolute_path_rejects_a_path_that_escapes_root() {
        let dir = scratch_dir("abs-path-escape");
        let outside = scratch_dir("abs-path-escape-outside-target");
        fs::write(outside.join("secret.txt"), "x").unwrap();
        let escaping_rel_path = format!("../{}/secret.txt", outside.file_name().unwrap().to_string_lossy());

        assert!(absolute_path(&dir, &escaping_rel_path).is_err());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }
}
