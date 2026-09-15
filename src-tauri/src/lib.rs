//! dsh-desktop: a Tauri shell that hosts the DeepSeek Harness web server
//! (`dsh web`) and points an embedded WebView2 at it.
//!
//! Security model: the harness page (http://127.0.0.1:<port>) is a plain
//! remote page and is never granted Tauri IPC access — `dangerousRemoteDomainIpcAccess`
//! is not enabled. All shell actions go through the native menu/tray and the
//! local boot page.
//!
//! Window layout: two webviews share the one window. The *shell* webview is
//! the config-created "main" (loads `tauri.localhost` → `ui/`; has IPC) and
//! draws the toolbar, boot/error states, dock, and all overlays. The *harness*
//! webview ([`HARNESS_WEBVIEW_LABEL`]) is a child created here in `setup`; it
//! navigates *top-level* to the ready URL dsh prints (which carries a required
//! `?token=`), so the harness is same-site with itself and its `SameSite=Strict`
//! `dsh-auth-*` cookie is actually stored and sent — the whole UI is behind that
//! browser auth, and an `<iframe>` under `tauri.localhost` is cross-site, so the
//! cookie could never take effect there and the window showed only the 401 line
//! (see `server::DSH_VERSION_DEFAULT`). Because it
//! loads an external origin, the harness webview gets no IPC *by construction*,
//! keeping the boundary above. `ui/app.js` positions it over the content
//! region (`harness_set_bounds`) and shows/hides it (`harness_show`/
//! `harness_hide`) as the server state and shell overlays change — a sibling
//! native webview would otherwise occlude the shell's own DOM overlays.

mod i18n;
mod menu;
mod panel;
mod server;
mod terminal;
#[cfg(windows)]
mod window_proc;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt as _, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

/// Ctrl+Alt+D (Cmd+Alt+D on macOS) — chosen to avoid the much more commonly
/// bound Ctrl+Space (IME switching) and Alt+Space (Windows' own system
/// menu). Summons the window from anywhere, same as a tray left-click.
fn global_show_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyD)
}

/// Pulls a usable workspace folder out of launch args — shared by a fresh
/// launch's own `std::env::args()` and the args a second launch attempt
/// hands to `tauri_plugin_single_instance`'s relaunch callback below. Skips
/// the first arg (the exe path itself); returns the first remaining arg
/// that resolves to an existing directory. This is the shape a Windows
/// Explorer "open with" shell command entry passes
/// (`"...\dsh-desktop.exe" "%V"`) — see server.rs's `DSH_DESKTOP_CWD` doc
/// comment for how it's consumed once resolved.
fn requested_workspace(args: &[String]) -> Option<std::path::PathBuf> {
    args.iter().skip(1).map(std::path::PathBuf::from).find(|p| p.is_dir())
}

use server::{DshServer, ServerStatus};

/// The user's remembered choice for what the window's close button (X)
/// should do, persisted across restarts once they've explicitly picked one
/// in the close-confirmation dialog and checked "remember". `None` — the
/// default, and also what a missing/corrupt settings file falls back to —
/// means "ask every time"; see `WindowEvent::CloseRequested` below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloseAction {
    Minimize,
    Quit,
}

/// Everything this shell remembers between launches. `#[serde(default)]` on
/// both the struct and each field means an older/partial file missing a key
/// still loads (that key just comes back `None`) rather than failing the
/// whole parse — adding a setting here never invalidates an existing file.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    close_action: Option<CloseAction>,
    /// Where the dsh server — and so the agent — is rooted by default.
    /// `None` means the user's home directory (the historical behavior).
    /// An explicit `DSH_DESKTOP_CWD` env var still wins over this: that's
    /// the per-launch override (a folder argument from Explorer's "open
    /// with", or the single-instance relaunch path), and a one-off "open
    /// this folder" should never silently overwrite what the user chose as
    /// their standing default.
    default_workspace: Option<PathBuf>,
}

/// Settings file — lives alongside the persistent desktop log
/// (`app_log_dir()`; confirmed to resolve correctly for both packaged and
/// unpackaged builds, unlike `app_cache_dir()` — see server.rs's
/// `runtime_dir` doc comment), not inside `~/.dsh`, which is dsh's own data
/// directory (a different project's domain — see this crate's module doc).
fn settings_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_log_dir().ok()
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    settings_dir(app).map(|dir| dir.join("settings.json"))
}

/// Reads persisted settings. Any failure (missing file, unreadable,
/// malformed JSON) is treated the same as "nothing saved yet" — these are
/// conveniences, not data worth failing startup or surfacing an error over.
///
/// Falls back to the pre-`settings.json` `close-action.json` file so a user
/// who already answered the close dialog (and checked "remember") before
/// this file existed keeps that answer instead of being asked again. The
/// fallback only applies when `settings.json` is absent/unreadable — a
/// readable `settings.json` that simply has no `closeAction` key means the
/// user never chose one, which is not the same as "the old file has it".
fn load_settings(app: &AppHandle) -> Settings {
    match settings_dir(app) {
        Some(dir) => load_settings_from(&dir),
        None => Settings::default(),
    }
}

/// Path-taking core of [`load_settings`], kept free of `AppHandle` so it can
/// be tested directly (same split as `panel.rs`'s `file_workspace_options`).
fn load_settings_from(dir: &std::path::Path) -> Settings {
    if let Ok(text) = fs::read_to_string(dir.join("settings.json")) {
        if let Ok(settings) = serde_json::from_str::<Settings>(&text) {
            return settings;
        }
    }
    match fs::read_to_string(dir.join("close-action.json")) {
        Ok(text) => Settings { close_action: serde_json::from_str(&text).ok(), ..Settings::default() },
        Err(_) => Settings::default(),
    }
}

/// Persists settings for future launches.
fn save_settings(app: &AppHandle, settings: &Settings) {
    let Some(path) = settings_path(app) else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(settings) {
        let _ = fs::write(path, text);
    }
}

pub struct AppState {
    pub server: Arc<Mutex<DshServer>>,
    /// Persisted preferences, loaded from disk at startup (see
    /// `load_settings`) and written back by whichever command changes one
    /// (`resolve_close_choice`, `set_default_workspace`).
    settings: Mutex<Settings>,
    /// The tray's "开机自动启动" checkbox — kept here so
    /// `MENU_TOGGLE_AUTOSTART` can sync its visual state after toggling
    /// (clicking a `CheckMenuItem` doesn't flip its own display automatically).
    autostart_item: CheckMenuItem<Wry>,
}

impl AppState {
    /// The saved default workspace, if any. Exposed for
    /// `server::workspace_dir`, which own the precedence rules — see its
    /// doc comment. Cloned out rather than returning the guard so callers
    /// never hold the settings lock while doing anything else with it.
    pub fn default_workspace(&self) -> Option<PathBuf> {
        self.settings.lock().unwrap().default_workspace.clone()
    }
}

// ── commands (called only from the local boot page) ─────────────────────────

/// Carries out the user's answer to the close-confirmation dialog (shown in
/// response to the `request-close-choice` event — see
/// `WindowEvent::CloseRequested`). `remember` persists the choice via
/// `save_settings` so future closes this run *and* future launches skip
/// the dialog entirely and just do it; leaving it unchecked means this
/// answer covers only the close that's happening right now.
#[tauri::command]
fn resolve_close_choice(app: AppHandle, quit: bool, remember: bool) {
    let action = if quit { CloseAction::Quit } else { CloseAction::Minimize };
    if remember {
        let state = app.state::<AppState>();
        let mut settings = state.settings.lock().unwrap();
        settings.close_action = Some(action);
        save_settings(&app, &settings);
    }
    match action {
        CloseAction::Minimize => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }
        }
        CloseAction::Quit => app.exit(0),
    }
}

/// The workspace the server is currently rooted in — what the settings
/// dialog pre-fills, and what the user is changing. Resolved the same way
/// `server::workspace_dir` does so the dialog always shows the *effective*
/// value rather than just the stored preference (which an explicit
/// `DSH_DESKTOP_CWD` can override for this launch).
#[tauri::command]
fn get_default_workspace(app: AppHandle, state: State<'_, AppState>) -> serde_json::Value {
    let override_active = std::env::var("DSH_DESKTOP_CWD").map(|v| !v.trim().is_empty()).unwrap_or(false);
    serde_json::json!({
        "effective": server::workspace_dir(&app).to_string_lossy(),
        "saved": state.settings.lock().unwrap().default_workspace.as_ref().map(|p| p.to_string_lossy().to_string()),
        // When set, the env var wins over the saved value for this launch —
        // the dialog says so, instead of appearing to contradict what it shows.
        "overrideActive": override_active,
    })
}

/// Sets (or with `null`, clears) the standing default workspace and restarts
/// the server so it takes effect immediately. `None` restores the historical
/// "user's home directory" behavior.
///
/// Rejects a path that isn't an existing directory: the value is persisted
/// and applied to every future launch, so silently storing a typo would look
/// like the setting simply not working, with nothing pointing at why.
#[tauri::command]
fn set_default_workspace(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let lang = i18n::detect();
    let workspace = match path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        Some(p) => {
            let pb = PathBuf::from(&p);
            if !pb.is_dir() {
                return Err(if lang == i18n::Lang::En {
                    format!("Not a directory, or it can't be read: {p}")
                } else {
                    format!("目录不存在或无法访问: {p}")
                });
            }
            Some(pb)
        }
        None => None,
    };
    {
        let state = app.state::<AppState>();
        let mut settings = state.settings.lock().unwrap();
        settings.default_workspace = workspace;
        save_settings(&app, &settings);
    }
    // The env var would otherwise keep shadowing the value just saved (see
    // `Settings::default_workspace`'s precedence note) — clearing it here is
    // what makes "set a new default" actually take effect for this run too,
    // rather than only after the next launch.
    std::env::remove_var("DSH_DESKTOP_CWD");
    let srv = app.state::<AppState>().server.clone();
    let app2 = app.clone();
    thread::spawn(move || server::restart(&app2, &srv));
    Ok(())
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> ServerStatus {
    state.server.lock().unwrap().status.clone()
}

#[tauri::command]
fn get_info(app: AppHandle, state: State<'_, AppState>) -> serde_json::Value {
    server::info(&app, &state.server)
}

#[tauri::command]
fn start_server(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let srv = state.server.clone();
    thread::spawn(move || {
        let _ = server::start(&app, &srv);
    });
    Ok(())
}

#[tauri::command]
fn restart_server(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let srv = state.server.clone();
    thread::spawn(move || server::restart(&app, &srv));
    Ok(())
}

#[tauri::command]
fn stop_server(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    server::stop(&state.server);
    server::set_stopped(&app, &state.server);
    Ok(())
}

/// `override_path`: the client's manual-picker choice, when it has one —
/// see the "known workspaces" section in `panel.rs`. `None`/absent falls
/// back to auto-inference, same as before that picker existed.
#[tauri::command]
fn get_workspace_tree(
    app: AppHandle,
    state: State<'_, AppState>,
    override_path: Option<String>,
) -> Vec<panel::TreeEntry> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::list_workspace_tree(&root)
}

#[tauri::command]
fn get_git_status(
    app: AppHandle,
    state: State<'_, AppState>,
    override_path: Option<String>,
) -> Vec<panel::GitEntry> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::git_status(&root)
}

/// The panel's workspace-name label when in auto mode. Re-resolved on every
/// panel refresh (not cached at startup like `get_info`'s other fields)
/// since the harness's own in-page workspace selection — entirely inside
/// the harness webview, with no signal reaching this shell directly — can change
/// independently of anything else this shell observes. See
/// `panel::active_workspace_dir`. Not called at all once the client has a
/// manual-picker choice locked in — it already knows what to show.
#[tauri::command]
fn get_active_workspace(app: AppHandle, state: State<'_, AppState>) -> String {
    panel::effective_workspace_dir(&app, &state.server).to_string_lossy().into_owned()
}

/// Every workspace dsh currently knows about — populates the panel's manual
/// picker. See the "known workspaces" section in `panel.rs`.
#[tauri::command]
fn get_known_workspaces(app: AppHandle, state: State<'_, AppState>) -> Vec<panel::WorkspaceOption> {
    panel::known_workspaces(&app, &state.server)
}

/// `path`: workspace-relative, as returned in a `TreeEntry`/`GitEntry`'s own
/// `path` field — the client passes through whichever tree row it clicked.
/// `override_path`: same manual-picker override as `get_workspace_tree`.
/// Returns both the current and (when there's a prior committed version
/// worth comparing against) `HEAD` content — `ui/app.js` hands both straight
/// to CodeMirror's `unifiedMergeView`, which does the actual diffing.
#[tauri::command]
fn get_editable_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    override_path: Option<String>,
) -> panel::EditablePreview {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::editable_preview(&root, &path)
}

#[tauri::command]
fn save_file_content(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    content: String,
    override_path: Option<String>,
) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::save_file(&root, &path, &content)
}

// ── tree context menu: file/folder operations ────────────────────────────
// Same override_path resolution as every command above — the manual
// workspace picker's choice when there is one, auto-inference otherwise.

#[tauri::command]
fn create_file(app: AppHandle, state: State<'_, AppState>, parent_path: String, name: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::create_file(&root, &parent_path, &name)
}

#[tauri::command]
fn create_dir(app: AppHandle, state: State<'_, AppState>, parent_path: String, name: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::create_dir(&root, &parent_path, &name)
}

#[tauri::command]
fn rename_entry(app: AppHandle, state: State<'_, AppState>, path: String, new_name: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::rename_entry(&root, &path, &new_name)
}

#[tauri::command]
fn move_entry(app: AppHandle, state: State<'_, AppState>, path: String, to_parent_path: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::move_entry(&root, &path, &to_parent_path)
}

#[tauri::command]
fn delete_entry(app: AppHandle, state: State<'_, AppState>, path: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::delete_entry(&root, &path)
}

#[tauri::command]
fn reveal_in_file_manager(app: AppHandle, state: State<'_, AppState>, path: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::reveal_in_file_manager(&app, &root, &path)
}

#[tauri::command]
fn open_with_default_app(app: AppHandle, state: State<'_, AppState>, path: String, override_path: Option<String>) -> Result<(), String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::open_with_default_app(&app, &root, &path)
}

#[tauri::command]
fn get_absolute_path(app: AppHandle, state: State<'_, AppState>, path: String, override_path: Option<String>) -> Result<String, String> {
    let root = override_path
        .map(PathBuf::from)
        .unwrap_or_else(|| panel::effective_workspace_dir(&app, &state.server));
    panel::absolute_path(&root, &path)
}

#[tauri::command]
fn get_log_tail(state: State<'_, AppState>, n: Option<usize>) -> Vec<String> {
    server::log_tail(&state.server, n.unwrap_or(100))
}

/// `package`: an npm package name or `github:owner/repo#path` source,
/// taken from a plugin entry in the catalog app.js fetches from
/// awesome-dsh-plugin.com (a crowd-submitted, unvetted list — see
/// PLUGIN_CATALOG_URL's doc comment there) after the user has confirmed
/// that exact entry in the plugin market dialog's confirm view. Never
/// anything sourced from the harness page itself, which stays outside this
/// shell's IPC boundary (see the module doc comment above) — only from
/// this shell's own UI, post-confirmation. Runs async: progress/completion
/// arrive via the `plugin-install` event, not this call's return value,
/// since the underlying `pnpm add` can take a while.
#[tauri::command]
fn install_plugin(app: AppHandle, state: State<'_, AppState>, package: String) -> Result<(), String> {
    server::install_plugin(&app, &state.server, &package)
}

/// Probed by the plugin market's confirm view before it enables "确认安装"
/// (see checkPnpmBeforeInstall in app.js) — `dsh plugin add` fails on a raw
/// "pnpm not found" OS error otherwise, which the confirm view has no way
/// to turn into a helpful message on its own.
#[tauri::command]
fn check_pnpm_available() -> bool {
    server::check_pnpm_available()
}

/// Runs `npm install -g pnpm` (see server::install_pnpm) — the fix the
/// confirm view offers inline when check_pnpm_available comes back false,
/// instead of just telling the user to go run it in a terminal themselves.
#[tauri::command]
fn install_pnpm(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    server::install_pnpm(&app, &state.server)
}

#[tauri::command]
fn open_in_browser(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let url = server::running_url(&state.server)
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", server::default_port()));
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a plugin's source repo (the "查看来源" button in the plugin market
/// dialog — see app.js's openConfirmView) in the system browser. Restricted
/// to https://github.com/ specifically, not a general-purpose URL opener:
/// `url` comes from the fetched awesome-dsh-plugin.com catalog, which this
/// project doesn't vet (see PLUGIN_CATALOG_URL's doc comment in app.js), so
/// this only ever hands the OS a link to the one place the whole feature is
/// already about — a plugin's own GitHub repo — never an arbitrary target.
#[tauri::command]
fn open_external_url(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://github.com/") {
        return Err(i18n::tr(i18n::detect(), "仅支持打开 GitHub 仓库链接。", "Only GitHub repository links can be opened.").to_string());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let home = server::dsh_home_dir(&app);
    let _ = std::fs::create_dir_all(&home);
    app.opener().reveal_item_in_dir(&home).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart_enabled(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Lets the in-window hamburger menu (Windows/Linux have no native menu bar —
/// see `set_menu()`'s macOS-only gate in `run()`) fire the exact same actions
/// as the tray menu, through the one dispatcher both already share.
#[tauri::command]
fn trigger_menu_action(app: AppHandle, id: String) {
    handle_menu_action(&app, &id);
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
    body: Option<String>,
}

/// Checks the configured updater endpoint for a newer shell release. Returns
/// `None` if the current version is already the latest (or the check
/// failed — network errors here shouldn't block the app from starting).
#[tauri::command]
async fn check_for_update(app: AppHandle) -> Option<UpdateInfo> {
    let update = app.updater().ok()?.check().await.ok()??;
    Some(UpdateInfo {
        version: update.version,
        body: update.body,
    })
}

/// Re-checks for an update and, if one is still available, downloads,
/// verifies (against the pinned pubkey) and installs it, then relaunches.
/// Re-checking here (rather than trusting a version string round-tripped
/// from `check_for_update`) avoids installing a stale/unverified `Update`.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| i18n::tr(i18n::detect(), "没有可用的更新", "No update available").to_string())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

// ── window helpers ───────────────────────────────────────────────────────────

/// Un-hide, un-minimize, and focus the main window. Used by both the tray's
/// "显示窗口" action and the single-instance relaunch callback below, so a
/// second launch attempt (desktop icon, Start menu, ...) surfaces the
/// existing window instead of spawning a second process/window/tray icon.
/// Callers include `tauri_plugin_single_instance`'s relaunch callback, which
/// fires on whatever thread delivers the second-instance IPC message — not
/// necessarily the main/event-loop thread window operations need to run on.
/// Confirmed by direct testing: called from that path, `win.show()` et al.
/// silently no-op (previously `let _ = ...`'d away with nothing logged) even
/// though the exact same window responds fine to an externally-driven
/// activation. `run_on_main_thread` dispatches back to the right thread
/// regardless of where this was called from, so every caller — tray click,
/// single-instance relaunch — gets the same, actually-working behavior.
fn show_main_window(app: &AppHandle) {
    let app_in_closure = app.clone();
    let result = app.run_on_main_thread(move || {
        let app = app_in_closure;
        if let Some(win) = app.get_webview_window("main") {
            if let Err(e) = win.show() {
                eprintln!("[dsh-desktop] show_main_window: show() failed: {e}");
            }
            if let Err(e) = win.unminimize() {
                eprintln!("[dsh-desktop] show_main_window: unminimize() failed: {e}");
            }
            if let Err(e) = win.set_focus() {
                eprintln!("[dsh-desktop] show_main_window: set_focus() failed: {e}");
            }
        } else {
            eprintln!("[dsh-desktop] show_main_window: no window labeled \"main\"");
        }
    });
    if let Err(e) = result {
        eprintln!("[dsh-desktop] show_main_window: run_on_main_thread failed: {e}");
    }
}

/// Single-instance relaunch path (second launch attempt while the app is
/// already running). Unlike the tray/hotkey callers, this is *not* a user
/// summoning the window on purpose — it fires for any duplicate launch,
/// including one this app didn't ask for (a stale launcher, a file
/// association, an autostart job). Showing + focusing unconditionally then
/// yanks the window to the front and steals focus from whatever the user is
/// doing in another app.
///
/// So: only summon the window when it isn't already on screen. If it is
/// visible and not minimized, leave it completely alone. If it was hidden
/// to the tray or minimized, surface it normally — that's what a user who
/// re-launches the app wants. Same thread discipline as `show_main_window`
/// (window APIs must run on the main/event-loop thread).
fn show_main_window_on_relaunch(app: &AppHandle) {
    let app_in_closure = app.clone();
    let result = app.run_on_main_thread(move || {
        let app = app_in_closure;
        if let Some(win) = app.get_webview_window("main") {
            match (win.is_visible(), win.is_minimized()) {
                (Ok(true), Ok(false)) => {
                    // Already on screen — do nothing, don't steal focus.
                }
                _ => show_main_window(&app),
            }
        } else {
            eprintln!("[dsh-desktop] show_main_window_on_relaunch: no window labeled \"main\"");
        }
    });
    if let Err(e) = result {
        eprintln!("[dsh-desktop] show_main_window_on_relaunch: run_on_main_thread failed: {e}");
    }
}

/// Disables WebView2's built-in right-click context menu (Back / Forward /
/// Reload / Inspect / …). The harness page is a plain remote page with no
/// Tauri-side navigation UI of its own, so that default menu was the only
/// way a user could reach browser-style back/forward — and "back" lands on
/// the local boot page with no way back to the harness UI short of a
/// restart (there is no in-app forward affordance, and the boot page's own
/// readiness check does not re-run on a history navigation). Removing the
/// menu removes the discoverable path into that dead end. `Settings` lives
/// on the `CoreWebView2` instance, not the page, so this only needs to run
/// once — it stays in effect across every later `navigate()` call (both to
/// the harness URL and back to the boot page on stop/restart).
#[cfg(windows)]
/// Removes only the context-menu items that caused real problems — not the
/// whole menu. An earlier version called
/// `SetAreDefaultContextMenusEnabled(false)`, which does disable the
/// back/forward navigation-trap this exists to prevent, but that's the same
/// menu Copy/Cut/Paste/Select All live on — nuking it took basic clipboard
/// interaction out with it. `ContextMenuRequested` lets the menu items be
/// inspected and selectively removed before it's shown, so "back"/"forward"
/// (the navigation trap) and "reload"/"inspectElement" (would reload the
/// remote harness page out from under its own client-side state, and
/// re-open the DevTools access `disable_devtools` closes below) go, while
/// everything else — Copy chief among them — stays.
fn disable_context_menu(win: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Controller, ICoreWebView2_11};
    use windows::core::{Interface, PWSTR};
    let _ = win.with_webview(|webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: windows::core::Result<()> = (|| unsafe {
            let core = controller.CoreWebView2()?;
            let core11: ICoreWebView2_11 = core.cast()?;
            let mut token = Default::default();
            core11.add_ContextMenuRequested(
                &webview2_com::ContextMenuRequestedEventHandler::create(Box::new(
                    move |_sender, args| {
                        let Some(args) = args else { return Ok(()) };
                        let items = args.MenuItems()?;
                        let mut count = 0u32;
                        items.Count(&mut count)?;
                        // Remove back-to-front: RemoveValueAtIndex shifts
                        // every later index down by one, so removing
                        // forward-to-back would skip whatever lands on an
                        // already-visited index.
                        for i in (0..count).rev() {
                            let item = items.GetValueAtIndex(i)?;
                            let mut name_ptr = PWSTR::null();
                            item.Name(&mut name_ptr)?;
                            let name = name_ptr.to_string().unwrap_or_default();
                            if matches!(name.as_str(), "back" | "forward" | "reload" | "inspectElement") {
                                items.RemoveValueAtIndex(i)?;
                            }
                        }
                        Ok(())
                    },
                )),
                &mut token,
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to trim WebView2 context menu: {e}");
        }
    });
}
#[cfg(not(windows))]
fn disable_context_menu(_win: &tauri::Webview) {}

#[cfg(windows)]
/// Embedded WebView2 controls (no visible browser chrome to show a
/// permission prompt) default every permission-gated Web API — clipboard
/// included — to Deny unless the host explicitly grants it here. The
/// harness page's own message Copy button calls the standard
/// `navigator.clipboard.writeText()`; under that default it rejects, and
/// the page's own caller swallows the rejection silently (`if (!ok)
/// return;`), which just looks like the button does nothing (see GitHub
/// issue #18). This is the fix on the host side, not a workaround on the
/// page — the page has no way to know it's running inside a host that
/// never grants what a normal browser tab would. Grants only
/// CLIPBOARD_READ (the WebView2 SDK's single permission kind gating the
/// whole clipboard API surface, both read and write, despite the name) —
/// every other kind (camera, mic, geolocation, notifications, ...) is left
/// unhandled, which resolves to WebView2's own default Deny. Same
/// one-time-at-setup reasoning as `disable_context_menu`: a
/// `CoreWebView2`-level event registration, not a per-page one.
fn allow_clipboard_permission(win: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    };
    let _ = win.with_webview(|webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: windows::core::Result<()> = (|| unsafe {
            let core = controller.CoreWebView2()?;
            let mut token = Default::default();
            core.add_PermissionRequested(
                &webview2_com::PermissionRequestedEventHandler::create(Box::new(
                    move |_sender, args| {
                        let Some(args) = args else { return Ok(()) };
                        let mut kind = Default::default();
                        args.PermissionKind(&mut kind)?;
                        if kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ {
                            args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                        }
                        Ok(())
                    },
                )),
                &mut token,
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to grant WebView2 clipboard permission: {e}");
        }
    });
}
#[cfg(not(windows))]
fn allow_clipboard_permission(_win: &tauri::Webview) {}

/// Disables Ctrl+scroll / Ctrl+±/0 page zoom entirely. Nothing in this app's
/// own UI (boot page or tray) exposes a zoom control — it's purely the
/// WebView2 accelerator-key default, which had no upper bound and no way
/// back to 100% short of restarting the app (Ctrl+0 didn't reset it; a
/// zoomed-out-to-nothing or zoomed-in-past-usable state persisted for the
/// rest of the session). Same one-time-at-setup reasoning as
/// `disable_context_menu`: this is a `CoreWebView2` setting, not a per-page
/// one, so it holds across every later `navigate()` call.
#[cfg(windows)]
fn disable_zoom_control(win: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    let _ = win.with_webview(|webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: windows::core::Result<()> = (|| unsafe {
            let core = controller.CoreWebView2()?;
            let settings = core.Settings()?;
            settings.SetIsZoomControlEnabled(false)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to disable WebView2 zoom control: {e}");
        }
    });
}
#[cfg(not(windows))]
fn disable_zoom_control(_win: &tauri::Webview) {}

/// Disables WebView2's built-in DevTools (F12 / Ctrl+Shift+I / right-click →
/// 检查) in release builds only — debug builds keep it, since it's the
/// normal way to debug the harness page during development. Release is a
/// consumer-facing build: an unlabeled "检查" entry (now moot, since the
/// context menu is off — but the keyboard shortcuts reach DevTools
/// independently of the context menu) would only confuse a non-technical
/// user who triggers it by accident, and leaving it reachable in a shipped
/// build is needless extra attack surface for no product benefit here.
#[cfg(all(windows, not(debug_assertions)))]
fn disable_devtools(win: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    let _ = win.with_webview(|webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: windows::core::Result<()> = (|| unsafe {
            let core = controller.CoreWebView2()?;
            let settings = core.Settings()?;
            settings.SetAreDevToolsEnabled(false)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to disable WebView2 DevTools: {e}");
        }
    });
}
#[cfg(not(all(windows, not(debug_assertions))))]
fn disable_devtools(_win: &tauri::Webview) {}

/// Message the injected file-mention script posts over the WebView2 host
/// channel (see [`wire_harness_file_mentions`]).
#[cfg(windows)]
#[derive(serde::Deserialize)]
struct FileMentionMsg {
    source: String,
    #[serde(rename = "type")]
    kind: String,
    path: String,
}

/// Wires the harness's file-mention buttons ("打开 <path>") to this shell's
/// file dock. The harness renders in its own top-level webview with no Tauri
/// IPC (see the module doc comment) and ships no `postMessage` channel of its
/// own; its source (`deepseek-ai/deepseek-harness`) isn't part of this repo,
/// so there's no click handler to edit directly. Two WebView2-level hooks on
/// the *harness* webview's own controller do it:
///
///  1. `AddScriptToExecuteOnDocumentCreated` injects a capture-phase click
///     interceptor that recognizes those buttons by their `aria-label`/`title`
///     shape (a "打开 <path>" label alongside a `title` holding that absolute
///     path — matched on the user-facing label, not the bundler-hashed CSS
///     class that changes every harness rebuild) and forwards the path via
///     `window.chrome.webview.postMessage`. That is the WebView2 *host*
///     channel, distinct from Tauri IPC: the harness has none, but this native
///     channel is always present and nothing else in that webview uses it.
///  2. `add_WebMessageReceived` validates the message shape and re-emits it as
///     the Tauri `open-file-mention` event, which `ui/app.js` (in the
///     IPC-having shell webview) listens for. Nothing here grants the harness
///     any new capability — it only ever asks the shell to open something the
///     shell already may open.
///
/// (Before the top-level-webview migration this was one injection into the
/// shell webview relying on the harness being a child `<iframe>` and
/// `window.top.postMessage`; that shared frame tree is gone now.)
#[cfg(windows)]
fn wire_harness_file_mentions(app: &AppHandle, harness: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    use webview2_com::{AddScriptToExecuteOnDocumentCreatedCompletedHandler, WebMessageReceivedEventHandler};
    use windows::core::{HSTRING, PWSTR};

    const SCRIPT: &str = r#"(function () {
  document.addEventListener(
    "click",
    function (event) {
      var el = event.target;
      while (el && el !== document.body) {
        if (el.tagName === "BUTTON") {
          var label = el.getAttribute("aria-label") || "";
          var path = el.getAttribute("title") || "";
          if (label.indexOf("打开 ") === 0 && path) {
            event.preventDefault();
            event.stopPropagation();
            if (window.chrome && window.chrome.webview) {
              window.chrome.webview.postMessage(
                JSON.stringify({ source: "dsh-desktop", type: "open-file-mention", path: path }),
              );
            }
            return;
          }
        }
        el = el.parentElement;
      }
    },
    true,
  );
})();"#;

    let app = app.clone();
    let _ = harness.with_webview(move |webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: webview2_com::Result<()> = (|| {
            let core = unsafe { controller.CoreWebView2() }?;
            {
                // Fire-and-forget: registration is async in WebView2, but we
                // don't need the returned script id. Deliberately NOT
                // `wait_for_async_operation` — that blocks the calling thread
                // until the WebView2 async op completes, and setup runs on the
                // event-loop thread, so waiting there deadlocks the app before
                // it ever reaches `server::start` (symptom: window with no
                // harness, nothing on stdout, port never bound).
                let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(
                    Box::new(|_result, _id| Ok(())),
                );
                unsafe {
                    core.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(SCRIPT), &handler)?;
                }
            }
            let mut token = Default::default();
            unsafe {
                core.add_WebMessageReceived(
                    &WebMessageReceivedEventHandler::create(Box::new(move |_sender, args| {
                        let Some(args) = args else { return Ok(()) };
                        let mut msg = PWSTR::null();
                        args.TryGetWebMessageAsString(&mut msg)?;
                        let raw = msg.to_string().unwrap_or_default();
                        if let Ok(m) = serde_json::from_str::<FileMentionMsg>(&raw) {
                            if m.source == "dsh-desktop"
                                && m.kind == "open-file-mention"
                                && !m.path.is_empty()
                            {
                                let _ = app.emit("open-file-mention", m.path);
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to wire harness file-mention bridge: {e}");
        }
    });
}
#[cfg(not(windows))]
fn wire_harness_file_mentions(_app: &AppHandle, _harness: &tauri::Webview) {}

/// Routes every outbound `http(s)` link — `target="_blank"`/`window.open`
/// (`NewWindowRequested`) and plain top-level navigation
/// (`NavigationStarting`) alike — to the system's default browser instead of
/// the embedded WebView2. Without a `NewWindowRequested` handler, WebView2's
/// default popup blocker (`IsDefaultPopupBlockerEnabled`, on by default)
/// silently swallows `target="_blank"` clicks — the harness page renders its
/// message/document links that way, so external links otherwise do nothing
/// at all. Without a `NavigationStarting` handler, a plain (non-`_blank`)
/// outbound link would instead navigate the WebView's top level away from
/// the harness UI, with no in-app way back (see `disable_context_menu`'s
/// doc comment on why "back" is removed from the context menu). Handling
/// both here keeps that decision entirely shell-side — it never depends on
/// or routes through the dsh web server. Same one-time-at-setup reasoning as
/// `disable_context_menu`: `CoreWebView2` event registrations, not per-page
/// ones, so they hold across every later `navigate()` call.
///
/// `127.0.0.1`/`localhost` URLs are left alone in both handlers — that's the
/// harness's own page loading, not an outbound link.
#[cfg(windows)]
fn install_external_link_handlers(app: &AppHandle, win: &tauri::Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    use webview2_com::{take_pwstr, NavigationStartingEventHandler, NewWindowRequestedEventHandler};
    use windows::core::PWSTR;

    fn is_external_http(uri: &str) -> bool {
        (uri.starts_with("http://") || uri.starts_with("https://"))
            && !uri.contains("127.0.0.1")
            && !uri.contains("localhost")
    }

    let app_new_window = app.clone();
    let app_navigation = app.clone();
    let _ = win.with_webview(move |webview| {
        let controller: ICoreWebView2Controller = webview.controller();
        let result: windows::core::Result<()> = (|| unsafe {
            let core = controller.CoreWebView2()?;

            let mut token_new_window = Default::default();
            core.add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri_ptr = PWSTR::null();
                    args.Uri(&mut uri_ptr)?;
                    let uri = take_pwstr(uri_ptr);
                    if is_external_http(&uri) {
                        let _ = app_new_window.opener().open_url(uri, None::<&str>);
                        // Only mark handled for the request this actually
                        // acted on. Per WebView2's own NewWindowRequested
                        // docs, `Handled=true` with no `NewWindow` set isn't
                        // "block this popup the same as leaving it alone" —
                        // it hands the opener script a dummy window that
                        // never loads and is immediately closed, whereas
                        // `Handled=false` (the default) lets WebView2 open
                        // an ordinary popup that actually navigates. A
                        // same-origin (127.0.0.1) `window.open()` — e.g. the
                        // harness triggering a file download without
                        // navigating its own top-level page away — falls
                        // through this `if` untouched, and unconditionally
                        // calling SetHandled(true) below used to kill that
                        // navigation outright before it ever loaded, which
                        // silently ate the download.
                        args.SetHandled(true)?;
                    }
                    Ok(())
                })),
                &mut token_new_window,
            )?;

            let mut token_navigation = Default::default();
            core.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri_ptr = PWSTR::null();
                    args.Uri(&mut uri_ptr)?;
                    let uri = take_pwstr(uri_ptr);
                    if is_external_http(&uri) {
                        let _ = app_navigation.opener().open_url(uri, None::<&str>);
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut token_navigation,
            )?;

            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dsh-desktop] failed to install external link handlers: {e}");
        }
    });
}
#[cfg(not(windows))]
fn install_external_link_handlers(_app: &AppHandle, _win: &tauri::Webview) {}

// ── menu / tray actions ──────────────────────────────────────────────────────

fn handle_menu_action(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match id {
        menu::MENU_OPEN_BROWSER => {
            let url = server::running_url(&state.server)
                .unwrap_or_else(|| format!("http://127.0.0.1:{}", server::default_port()));
            let _ = app.opener().open_url(url, None::<&str>);
        }
        menu::MENU_RESTART => {
            let srv = state.server.clone();
            let app2 = app.clone();
            thread::spawn(move || server::restart(&app2, &srv));
        }
        menu::MENU_OPEN_DATA_DIR => {
            let home = server::dsh_home_dir(app);
            let _ = std::fs::create_dir_all(&home);
            let _ = app.opener().reveal_item_in_dir(&home);
        }
        menu::MENU_SET_WORKSPACE => {
            // Reachable from the tray while the window is hidden, so surface
            // the window first — the dialog this opens lives in the shell
            // page, and an event sent to a hidden window would just sit
            // there unread.
            show_main_window(app);
            let _ = app.emit("request-set-workspace", ());
        }
        menu::MENU_SHOW_WINDOW => show_main_window(app),
        menu::MENU_TOGGLE_AUTOSTART => {
            let autolaunch = app.autolaunch();
            let now_enabled = autolaunch.is_enabled().unwrap_or(false);
            let result = if now_enabled { autolaunch.disable() } else { autolaunch.enable() };
            match result {
                Ok(()) => {
                    let _ = state.autostart_item.set_checked(!now_enabled);
                }
                Err(e) => eprintln!("[dsh-desktop] failed to toggle autostart: {e}"),
            }
        }
        menu::MENU_QUIT => {
            // The spawned server (and any live terminal shell) is torn down
            // in the ExitRequested handler at the end of run() — the single
            // place covering every quit path (app-menu ⌘Q role, tray 退出,
            // updater restart).
            app.exit(0);
        }
        _ => {}
    }
}

// ── harness child webview ──────────────────────────────────────────────────
//
// The harness (dsh web UI) renders here, not in an <iframe> in the shell page,
// so it is the top-level document — the only arrangement in which dsh 0.1.5's
// `SameSite=Strict` auth cookie is stored and sent (see the module doc comment).
// Created hidden in `setup`; ui/app.js drives it through the commands below.
// Navigation is deduped on the JS side (it tracks the ready URL from the
// `server-status` event), which is why these stay stateless.

/// Label of the child webview that hosts the harness. Looked up by
/// `Manager::get_webview` in the commands below.
const HARNESS_WEBVIEW_LABEL: &str = "harness";

/// Position/size the harness webview over the shell's content region.
/// Coordinates are **physical pixels** relative to the window client area
/// (ui/app.js multiplies its CSS-pixel content rect by `devicePixelRatio`).
#[tauri::command]
fn harness_set_bounds(app: AppHandle, x: i32, y: i32, width: u32, height: u32) {
    if let Some(hv) = app.get_webview(HARNESS_WEBVIEW_LABEL) {
        let _ = hv.set_bounds(tauri::Rect {
            position: tauri::PhysicalPosition::new(x, y).into(),
            // A zero dimension can drop the WebView2 controller into a bad
            // state on some builds; never hand it one.
            size: tauri::PhysicalSize::new(width.max(1), height.max(1)).into(),
        });
    }
}

/// Navigate the harness webview to `url` (the ready URL from the server), then
/// show it. Only called when entering the running state or when the URL
/// changes — ui/app.js does the deduping, so this always navigates.
#[tauri::command]
fn harness_show(app: AppHandle, url: String) {
    if let Some(hv) = app.get_webview(HARNESS_WEBVIEW_LABEL) {
        if let Ok(parsed) = url.parse::<tauri::Url>() {
            let _ = hv.navigate(parsed);
        }
        let _ = hv.show();
    }
}

/// Just show the harness webview at its current URL, no navigation — used to
/// bring it back after a shell overlay that had hidden it closes, without
/// reloading (and losing) the harness page's state.
#[tauri::command]
fn harness_reveal(app: AppHandle) {
    if let Some(hv) = app.get_webview(HARNESS_WEBVIEW_LABEL) {
        let _ = hv.show();
    }
}

/// Hide the harness webview so the shell's own content (boot/error cards, or a
/// full-window overlay like the plugin market or a dialog) can paint over the
/// content region a sibling native webview would otherwise occlude.
#[tauri::command]
fn harness_hide(app: AppHandle) {
    if let Some(hv) = app.get_webview(HARNESS_WEBVIEW_LABEL) {
        let _ = hv.hide();
    }
}

// ── app entry ────────────────────────────────────────────────────────────────

pub fn run() {
    // A folder arg on the *first* launch (e.g. from a Windows Explorer
    // "open with" shell command entry) picks the workspace the server
    // starts in — set before anything reads DSH_DESKTOP_CWD. An explicit
    // env var the user already set wins over this, same precedence as every
    // other DSH_DESKTOP_* override.
    if std::env::var("DSH_DESKTOP_CWD").is_err() {
        let args: Vec<String> = std::env::args().collect();
        if let Some(dir) = requested_workspace(&args) {
            std::env::set_var("DSH_DESKTOP_CWD", &dir);
        }
    }

    // Created here rather than inside `.setup()` (its previous home) so the
    // panic hook below can close over it too — `.setup()` doesn't run until
    // the event loop is already live, which is too late to cover a panic
    // during the builder chain itself, however unlikely.
    let srv = Arc::new(Mutex::new(DshServer::default()));

    // Every *normal* quit path (window close, tray 退出, updater restart)
    // funnels through `RunEvent::ExitRequested` below, which tears the dsh
    // child process down. A panic doesn't: it unwinds past that entirely,
    // and if it happens to take down the event-loop thread, the child is
    // orphaned with zero cleanup. This is the safety net — best-effort, but
    // strictly better than nothing. Chains the previous hook (Tauri/the
    // default runtime's own) rather than replacing it, so panic messages
    // still get logged/reported the same as before.
    server::init_panic_cleanup(srv.clone());
    let previous_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        server::cleanup_on_panic();
        previous_panic_hook(info);
    }));

    tauri::Builder::default()
        // Must be the first plugin registered: it needs to claim the
        // single-instance lock before anything else in the builder chain
        // runs. A second launch (desktop icon, Start menu, ...) hits this
        // callback in the *first* process and exits immediately instead of
        // creating its own window/tray icon — see `show_main_window`.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A folder arg here (second launch — e.g. right-clicking a
            // *different* folder in Explorer while the app is already
            // running) switches the running server to that workspace.
            // Doesn't lose anything: dsh persists sessions per-workspace-
            // path under ~/.dsh/sessions/ regardless of which one the
            // server is currently pointed at, so this is "switch which
            // project's history you're looking at", not "discard unsaved
            // work" — restart() (stop + start) already re-reads
            // DSH_DESKTOP_CWD fresh on every call, so setting it here is
            // all switching the workspace takes.
            if let Some(dir) = requested_workspace(&args) {
                std::env::set_var("DSH_DESKTOP_CWD", &dir);
                let state = app.state::<AppState>();
                let srv = state.server.clone();
                let app2 = app.clone();
                thread::spawn(move || server::restart(&app2, &srv));
                // A workspace switch is a deliberate summon (the user just
                // picked a folder to open), same as the tray click/hotkey —
                // always surface it so they see the switch happen, even if
                // the window was technically "visible" but buried behind
                // other windows or on another virtual desktop.
                show_main_window(app);
            } else {
                // No workspace arg: an untargeted duplicate launch (desktop
                // icon, Start menu, or something this app didn't ask for —
                // a stale launcher, autostart job). Don't steal focus when
                // the window is already on screen — see
                // show_main_window_on_relaunch.
                show_main_window_on_relaunch(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state == ShortcutState::Pressed && *shortcut == global_show_shortcut() {
                        show_main_window(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            resolve_close_choice,
            get_default_workspace,
            set_default_workspace,
            get_status,
            get_info,
            start_server,
            restart_server,
            stop_server,
            get_log_tail,
            install_plugin,
            check_pnpm_available,
            install_pnpm,
            open_in_browser,
            open_external_url,
            open_data_dir,
            get_autostart_enabled,
            trigger_menu_action,
            check_for_update,
            install_update,
            get_workspace_tree,
            get_git_status,
            get_active_workspace,
            get_known_workspaces,
            get_editable_preview,
            save_file_content,
            create_file,
            create_dir,
            rename_entry,
            move_entry,
            delete_entry,
            reveal_in_file_manager,
            open_with_default_app,
            get_absolute_path,
            terminal::terminal_spawn,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            harness_set_bounds,
            harness_show,
            harness_reveal,
            harness_hide
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // Persistent log file for the desktop shell itself.
            if let Ok(log_dir) = app.path().app_log_dir() {
                server::init_log_file(log_dir.join("desktop.log"));
            }

            // `srv` itself (not a fresh one — `AppState` isn't `manage()`d
            // until below, once `build_tray` has handed back the autostart
            // checkbox it needs to be constructed with) is the same instance
            // the panic hook above already closed over, created before this
            // closure so both share one server rather than racing to set up
            // two independent ones.
            let settings = Mutex::new(load_settings(&handle));

            // WebView2-level settings for the *shell* webview (which draws the
            // toolbar, boot/error states, dock, and overlays). They hold for
            // its whole lifetime.
            //
            // These are all control-level (`CoreWebView2` event registrations /
            // settings), so they used to cover the harness too, back when it
            // was an <iframe> inside this webview's document tree. It is now a
            // separate child webview with its own controller, so the harness
            // needs the same set applied to *its* controller — done in the
            // creation block below. Miss that and the harness silently loses
            // context-menu suppression, zoom lock, the clipboard grant, and
            // (worst) external-link routing, which would leave `target="_blank"`
            // links swallowed by WebView2's popup blocker and plain outbound
            // links navigating the harness away with no way back.
            if let Some(win) = app.get_webview_window("main") {
                let shell: &tauri::Webview = win.as_ref();
                disable_context_menu(shell);
                disable_zoom_control(shell);
                disable_devtools(shell);
                allow_clipboard_permission(shell);
                install_external_link_handlers(&handle, shell);
                // Windows-only: makes the maximized window's top strip
                // deliver hover/click to the web content instead of a
                // resize cursor — see window_proc.rs. Window-level (not a
                // webview setting), so it stays on the WebviewWindow.
                #[cfg(windows)]
                window_proc::patch_maximized_hit_test(&win);
            }

            // The harness webview (see the module doc comment): a child of the
            // main window that ui/app.js navigates top-level to the dsh URL.
            // Starts at about:blank and hidden; app.js positions it over the
            // content region and shows it once the server reports Running.
            // Loading an external origin, it never receives Tauri IPC — the
            // boundary we mean to keep. Initial bounds are a throwaway (it is
            // hidden until app.js sends real ones via `harness_set_bounds`).
            if let Some(win) = app.get_window("main") {
                match win.add_child(
                    tauri::webview::WebviewBuilder::new(
                        HARNESS_WEBVIEW_LABEL,
                        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                    ),
                    tauri::LogicalPosition::new(0.0, 0.0),
                    tauri::LogicalSize::new(320.0, 240.0),
                ) {
                    Ok(harness) => {
                        let _ = harness.hide();
                        // The same control-level WebView2 settings the shell
                        // gets above — the harness is its own controller now,
                        // so it no longer inherits them from the shell's (see
                        // that block's comment; `install_external_link_handlers`
                        // and `allow_clipboard_permission` matter most here,
                        // being specifically about the harness page's links and
                        // Copy buttons). No-ops off Windows.
                        disable_context_menu(&harness);
                        disable_zoom_control(&harness);
                        disable_devtools(&harness);
                        allow_clipboard_permission(&harness);
                        install_external_link_handlers(&handle, &harness);
                        // Route the harness's file-mention buttons into the
                        // file dock — see `wire_harness_file_mentions`.
                        wire_harness_file_mentions(&handle, &harness);
                    }
                    Err(e) => eprintln!("[dsh-desktop] failed to create harness webview: {e}"),
                }
            }

            // `decorations: false` in tauri.conf.json gives the frameless
            // Windows/Linux look (custom in-page buttons + drag region), but
            // macOS users expect the native title bar — real traffic lights
            // on the left and drag-to-move for free. Restore it there; the
            // shell page mirrors this compile-time platform split in
            // ui/app.js (IS_MACOS) by hiding its custom window controls and
            // dropping the toolbar drag region.
            #[cfg(target_os = "macos")]
            if let Some(win) = app.get_webview_window("main") {
                win.set_decorations(true)?;
            }

            // A window/app menu set via `set_menu()` becomes the global
            // top-of-screen menu bar on macOS (platform convention, doesn't
            // cost window space) but a classic in-window Win32-style menu
            // strip on Windows/Linux — stacked right under the native title
            // bar, i.e. two layers of chrome for one "文件" entry. Every
            // action it offered already lives in the tray menu below, so
            // only set it on macOS.
            #[cfg(target_os = "macos")]
            app.set_menu(menu::build_menu(&handle)?)?;
            let autostart_item = menu::build_tray(&handle, handle_menu_action)?;

            // Best-effort: another app may already hold Ctrl+Alt+D. Not
            // being able to summon the window by hotkey isn't worth failing
            // startup over — the tray icon still works either way.
            if let Err(e) = app.global_shortcut().register(global_show_shortcut()) {
                eprintln!("[dsh-desktop] failed to register global shortcut: {e}");
            }

            app.manage(AppState {
                server: srv.clone(),
                settings,
                autostart_item,
            });
            app.manage(terminal::TerminalState::default());

            // Auto-start the harness server shortly after the window appears.
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(300));
                let _ = server::start(&handle, &srv);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Always intercept the raw OS-level close — what actually
                // happens next (minimize to tray, or quit) is decided below,
                // never by letting this proceed unprompted.
                api.prevent_close();
                let app = window.app_handle();
                let remembered = app.state::<AppState>().settings.lock().unwrap().close_action;
                match remembered {
                    Some(CloseAction::Minimize) => {
                        let _ = window.hide();
                    }
                    Some(CloseAction::Quit) => {
                        // Quitting (⌘Q / tray 退出 / a remembered "quit" here)
                        // all tear the server down in the ExitRequested
                        // handler below and then exit the process.
                        app.exit(0);
                    }
                    // No remembered choice yet: ask, via a dialog on the boot
                    // page — resolve_close_choice carries out whatever the
                    // user picks. Don't hide or quit here; either would race
                    // ahead of the user's actual answer.
                    None => {
                        let _ = app.emit("request-close-choice", ());
                    }
                }
            }
        })
        .on_menu_event(|app, event| handle_menu_action(app, event.id.as_ref()))
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Tear down the spawned dsh server and any live terminal shell
            // before the process exits — otherwise they're orphaned and
            // keep running/serving. Every quit path (the app-menu ⌘Q role
            // on macOS, the tray "退出" item, updater restarts) funnels into
            // ExitRequested before the event loop ends, so this is the one
            // place it needs to happen.
            if let RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app.try_state::<AppState>() {
                    server::stop(&state.server);
                }
                if let Some(term_state) = app.try_state::<terminal::TerminalState>() {
                    terminal::kill(&term_state);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{load_settings_from, requested_workspace, CloseAction, Settings};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn finds_first_existing_dir_after_the_exe_path() {
        let tmp = std::env::temp_dir();
        let args = vec!["dsh-desktop.exe".to_string(), tmp.display().to_string()];
        assert_eq!(requested_workspace(&args), Some(tmp));
    }

    #[test]
    fn ignores_a_nonexistent_path() {
        let args = vec![
            "dsh-desktop.exe".to_string(),
            "Z:\\definitely\\not\\a\\real\\path\\hopefully".to_string(),
        ];
        assert_eq!(requested_workspace(&args), None);
    }

    #[test]
    fn no_args_beyond_the_exe_path_finds_nothing() {
        let args = vec!["dsh-desktop.exe".to_string()];
        assert_eq!(requested_workspace(&args), None);
    }

    #[test]
    fn skips_the_exe_path_itself_even_if_its_directory_exists() {
        // The exe's own directory is a real, existing dir — make sure we
        // never mistake arg[0] (the exe path, not a workspace request) for
        // a workspace request just because its parent happens to exist.
        let tmp = std::env::temp_dir();
        let fake_exe = tmp.join("dsh-desktop.exe");
        let args = vec![fake_exe.display().to_string()];
        assert_eq!(requested_workspace(&args), None);
    }

    // ── settings persistence ──

    /// Unique per-test scratch directory, cleaned up on the way in (a
    /// previous failed run may have left one behind). Same pattern as
    /// `panel.rs`'s tests.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-desktop-settings-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_load_default_when_nothing_was_ever_saved() {
        let dir = scratch_dir("empty");
        let settings = load_settings_from(&dir);
        assert!(settings.close_action.is_none());
        assert!(settings.default_workspace.is_none());
    }

    #[test]
    fn settings_round_trip_through_the_new_file() {
        let dir = scratch_dir("roundtrip");
        let saved = Settings {
            close_action: Some(CloseAction::Quit),
            default_workspace: Some(PathBuf::from(r"C:\projects\thing")),
        };
        fs::write(dir.join("settings.json"), serde_json::to_string(&saved).unwrap()).unwrap();

        let loaded = load_settings_from(&dir);
        assert_eq!(loaded.close_action, Some(CloseAction::Quit));
        assert_eq!(loaded.default_workspace, Some(PathBuf::from(r"C:\projects\thing")));
    }

    #[test]
    fn settings_fall_back_to_the_legacy_close_action_file() {
        // A user who answered the close dialog before settings.json existed
        // must keep that answer rather than being asked all over again.
        let dir = scratch_dir("legacy");
        fs::write(dir.join("close-action.json"), r#""minimize""#).unwrap();

        let loaded = load_settings_from(&dir);
        assert_eq!(loaded.close_action, Some(CloseAction::Minimize));
        assert!(loaded.default_workspace.is_none());
    }

    #[test]
    fn settings_prefer_the_new_file_over_the_legacy_one() {
        // Both present: settings.json is the current source of truth, and a
        // missing closeAction key in it means "never chose one" — not "fall
        // back to the stale legacy file".
        let dir = scratch_dir("prefer-new");
        fs::write(dir.join("close-action.json"), r#""minimize""#).unwrap();
        fs::write(dir.join("settings.json"), r#"{"defaultWorkspace":"C:\\projects\\thing"}"#).unwrap();

        let loaded = load_settings_from(&dir);
        assert!(loaded.close_action.is_none());
        assert_eq!(loaded.default_workspace, Some(PathBuf::from(r"C:\projects\thing")));
    }

    #[test]
    fn settings_survive_a_malformed_file_instead_of_failing_startup() {
        let dir = scratch_dir("malformed");
        fs::write(dir.join("settings.json"), "{ this is not json").unwrap();

        // Falls through to "nothing saved" rather than panicking or erroring.
        let loaded = load_settings_from(&dir);
        assert!(loaded.close_action.is_none() && loaded.default_workspace.is_none());
    }

    #[test]
    fn settings_tolerate_a_file_missing_newer_keys() {
        // A settings.json written by an older build (before a key existed)
        // must still load the keys it does have.
        let dir = scratch_dir("partial");
        fs::write(dir.join("settings.json"), r#"{"closeAction":"quit"}"#).unwrap();

        let loaded = load_settings_from(&dir);
        assert_eq!(loaded.close_action, Some(CloseAction::Quit));
        assert!(loaded.default_workspace.is_none());
    }
}
