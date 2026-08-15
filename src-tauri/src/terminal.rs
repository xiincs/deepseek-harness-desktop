//! Backing process for the toolbar terminal button: a single, singleton PTY
//! shell (not one per some future multi-tab UI — nothing here asks for that
//! yet). Spawned lazily on first open and kept alive across the dock card
//! being hidden/shown again, same lifetime as `AppState`'s own server handle.

use std::io::{Read, Write};
use std::sync::Mutex;

use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter, State};

use crate::panel;
use crate::AppState;

struct TerminalSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalState(Mutex<Option<TerminalSession>>);

/// `cmd.exe` via `%COMSPEC%` — present on every Windows install without
/// depending on PowerShell being on `PATH`; a user who wants pwsh can just
/// type it once the shell is up.
fn shell_command() -> CommandBuilder {
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    CommandBuilder::new(shell)
}

/// Reads PTY output on a background thread for as long as the session
/// outlives this call, base64-encoding each chunk (raw bytes can split a
/// multi-byte UTF-8 sequence or contain control bytes a JSON string can't
/// carry) and forwarding it to the dock card via the `terminal-data` event.
/// EOF (child exited, pipe closed) fires `terminal-exit` once and returns.
fn spawn_reader(app: AppHandle, mut reader: Box<dyn Read + Send>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = app.emit("terminal-exit", ());
                    return;
                }
                Ok(n) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = app.emit("terminal-data", encoded);
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
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut guard = term_state.0.lock().unwrap();
    // Idempotent: reopening the card after just hiding it (not closing it)
    // should reattach to the still-running shell, not spawn a second one on
    // top of it — only a genuinely dead session (or none yet) gets replaced.
    if let Some(session) = guard.as_mut() {
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

    *guard = Some(TerminalSession { writer, master: pair.master, child });
    drop(guard);

    spawn_reader(app, reader);
    Ok(())
}

#[tauri::command]
pub fn terminal_write(term_state: State<'_, TerminalState>, data: String) -> Result<(), String> {
    let mut guard = term_state.0.lock().unwrap();
    let session = guard.as_mut().ok_or("no terminal session")?;
    session.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_resize(term_state: State<'_, TerminalState>, cols: u16, rows: u16) -> Result<(), String> {
    let guard = term_state.0.lock().unwrap();
    let session = guard.as_ref().ok_or("no terminal session")?;
    session
        .master
        .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_close(term_state: State<'_, TerminalState>) -> Result<(), String> {
    kill(&term_state);
    Ok(())
}

/// Shared by the `terminal_close` command and the app's real quit path
/// (`MENU_QUIT` in lib.rs — `CloseRequested` only hides to tray and leaves
/// this running, same as it leaves the dsh server running).
pub fn kill(term_state: &TerminalState) {
    if let Some(mut session) = term_state.0.lock().unwrap().take() {
        let _ = session.child.kill();
    }
}
