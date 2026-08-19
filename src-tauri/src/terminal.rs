//! Backing processes for the toolbar terminal button's tabs: one PTY shell
//! per open tab, keyed by an id the client hands in (see `ui/app.js`'s
//! terminalTabs map — this side never generates ids itself, it just uses
//! whatever the client sends as a HashMap key). Each tab is spawned lazily
//! when its own tab is created and kept alive across the dock card being
//! hidden/shown again, same lifetime as `AppState`'s own server handle —
//! only closing a specific tab, or the app quitting, kills that tab's shell.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::panel;
use crate::AppState;

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalState(Mutex<HashMap<u32, TerminalSession>>);

#[derive(Clone, Serialize)]
struct TerminalDataEvent {
    id: u32,
    data: String,
}

#[derive(Clone, Serialize)]
struct TerminalExitEvent {
    id: u32,
}

/// The shell the dock terminal runs.
///
/// Windows: `cmd.exe` via `%COMSPEC%` — present on every Windows install
/// without depending on PowerShell being on `PATH`; a user who wants pwsh
/// can just type it once the shell is up.
///
/// macOS/Linux: the user's login shell (`$SHELL`), run with `-l` so the
/// profile chain executes (`path_helper` on macOS pulls Homebrew etc. into
/// `PATH` — the GUI app itself inherits the minimal Finder PATH). Spawning
/// `cmd.exe` here is what produced "Unable to spawn cmd.exe…" on macOS.
#[cfg(target_os = "windows")]
fn shell_command() -> CommandBuilder {
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    CommandBuilder::new(shell)
}

#[cfg(not(target_os = "windows"))]
fn shell_command() -> CommandBuilder {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".into()
            } else {
                "/bin/bash".into()
            }
        });
    let mut cmd = CommandBuilder::new(shell);
    cmd.arg("-l");
    cmd
}

/// Reads PTY output on a background thread for as long as the session
/// outlives this call, base64-encoding each chunk (raw bytes can split a
/// multi-byte UTF-8 sequence or contain control bytes a JSON string can't
/// carry) and forwarding it to the dock card via the `terminal-data` event,
/// tagged with `id` so the client can route it to the right tab's xterm.js
/// instance — every tab's reader thread emits on the same global event name,
/// there's no per-tab event channel. EOF (child exited, pipe closed) fires
/// `terminal-exit` once (same `id` tag) and returns.
fn spawn_reader(app: AppHandle, id: u32, mut reader: Box<dyn Read + Send>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = app.emit("terminal-exit", TerminalExitEvent { id });
                    return;
                }
                Ok(n) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = app.emit("terminal-data", TerminalDataEvent { id, data: encoded });
                }
            }
        }
    });
}

#[tauri::command]
pub fn terminal_spawn(
    app: AppHandle,
    state: State<'_, AppState>,
    term_state: State<'_, TerminalState>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut guard = term_state.0.lock().unwrap();
    // Idempotent: reopening the card after just hiding it (not closing it)
    // should reattach to the still-running shell, not spawn a second one on
    // top of it — only a genuinely dead session (or none yet) gets replaced.
    // In practice the client only calls this once per id (its own `spawned`
    // flag guards re-calling it for a tab already known to be live) — this
    // is a backstop against client/server state ever drifting apart, not the
    // primary guard.
    if let Some(session) = guard.get_mut(&id) {
        if matches!(session.child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }

    let cwd = panel::effective_workspace_dir(&app, &state.server);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;

    let mut cmd = shell_command();
    cmd.cwd(&cwd);
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    // The slave end belongs to the child process now; holding it open past
    // this point only pins a duplicate handle to the same pty on Windows.
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    guard.insert(id, TerminalSession { writer, master: pair.master, child });
    drop(guard);

    spawn_reader(app, id, reader);
    Ok(())
}

#[tauri::command]
pub fn terminal_write(term_state: State<'_, TerminalState>, id: u32, data: String) -> Result<(), String> {
    let mut guard = term_state.0.lock().unwrap();
    let session = guard.get_mut(&id).ok_or("no terminal session")?;
    session.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_resize(term_state: State<'_, TerminalState>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let guard = term_state.0.lock().unwrap();
    let session = guard.get(&id).ok_or("no terminal session")?;
    session
        .master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_close(term_state: State<'_, TerminalState>, id: u32) -> Result<(), String> {
    kill_one(&term_state, id);
    Ok(())
}

/// `session.child.kill()` alone only reaches the shell itself, not whatever
/// it launched interactively (an npm/python/nested-shell process) — those
/// would otherwise survive as orphans. Fall back to the direct kill only if
/// we can't get a pid to hand to the tree-kill (portable_pty::Child::process_id
/// can return None, e.g. once the process has already exited on its own).
fn kill_session(mut session: TerminalSession) {
    match session.child.process_id() {
        Some(pid) => crate::server::kill_process_tree(pid),
        None => {
            let _ = session.child.kill();
        }
    }
}

/// Shared by the `terminal_close` command (one tab's own × button) and
/// restarting a tab (close then immediately re-spawn the same id).
pub fn kill_one(term_state: &TerminalState, id: u32) {
    if let Some(session) = term_state.0.lock().unwrap().remove(&id) {
        kill_session(session);
    }
}

/// Every open tab at once — for the app's real quit path (`MENU_QUIT` in
/// lib.rs — `CloseRequested` only hides to tray and leaves this running,
/// same as it leaves the dsh server running), which by then has no reason to
/// know or care what tab ids exist client-side. The dock's own "close all
/// tabs" header button doesn't call this: it still needs its own client-side
/// loop over its tab map either way to dispose each xterm.js instance and
/// its DOM, so it just calls terminal_close per id from there rather than
/// this app also growing a bulk-close command redundant with that loop.
pub fn kill(term_state: &TerminalState) {
    let sessions: Vec<TerminalSession> = term_state.0.lock().unwrap().drain().map(|(_, session)| session).collect();
    for session in sessions {
        kill_session(session);
    }
}
