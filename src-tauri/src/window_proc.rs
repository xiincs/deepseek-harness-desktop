//! Windows-only native window-procedure patches for the frameless main
//! window. Everything here exists because a strip of pixels at the very
//! top of a *maximized* frameless window is owned by native hit-testing,
//! not by the web content — see `patch_maximized_hit_test` for the full
//! mechanism and why no amount of CSS/JS can fix it.

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT, HTLEFT, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, IsZoomed, ShowWindow, SW_HIDE, SW_SHOW, WM_NCHITTEST, WM_SIZE,
};

/// tauri-runtime-wry (pinned in Cargo.lock; this project tracks tauri 2.x
/// via `tauri = "2"`) creates an invisible child HWND over every
/// undecorated, resizable window to provide native edge-resizing —
/// `undecorated_resizing.rs::attach_resize_handler`. The child is named
/// `TAURI_DRAG_RESIZE_WINDOW` under class `TAURI_DRAG_RESIZE_BORDERS`, and
/// its own window proc answers WM_NCHITTEST with HTTOP/HTLEFT/… for the
/// border strip. Those class/window names are internal implementation
/// details of tauri-runtime-wry, not a public API: if an upgrade ever
/// stops matching, this module silently degrades to a no-op (nothing here
/// is load-bearing for the rest of the app) but the maximized-window strip
/// bug below comes back, so re-check `undecorated_resizing.rs` on upgrade.
const TAURI_DRAG_RESIZE_CLASS: windows::core::PCWSTR = w!("TAURI_DRAG_RESIZE_BORDERS");
const TAURI_DRAG_RESIZE_NAME: windows::core::PCWSTR = w!("TAURI_DRAG_RESIZE_WINDOW");

const SUBCLASS_ID_PARENT: usize = 0x4453_4801; // "DSH" …01

/// Fixes hit-testing quirks of maximized frameless windows on Windows.
///
/// The frameless window keeps the thick-frame window styles
/// (WS_CAPTION|WS_SIZEBOX — see tao's `to_window_styles`) so resizing
/// works, and tauri-runtime-wry adds the drag-resize child above for the
/// actual edge dragging. Both hit-test paths only make sense while the
/// window can *be* resized, but neither properly deactivates when the
/// window is maximized:
///
/// - tauri-runtime-wry's own WM_SIZE handler does try to collapse the
///   drag-resize child to 0×0 while maximized, but only when a WM_SIZE
///   arrives *after* the child was attached. This window starts maximized
///   (`maximized: true` in tauri.conf.json), and the child is attached
///   after the window is already fully created and maximized — the
///   maximize WM_SIZE fired before the attach, so the child stays at full
///   client size with its border-strip region, answering HTTOP for the
///   top strip of the screen even though the window can't be resized.
/// - the parent's DefWindowProc still reports border hits for the
///   window's edges; for a maximized window the top border sits exactly
///   on the top row of the work area.
///
/// Result: the topmost row(s) of a maximized window get hit-tested as a
/// resize border. Windows shows the vertical-resize cursor, and while a
/// *move* over that strip can be routed onward (a transparent hit test
/// passes it to the WebView2 beneath — hover highlight worked), a *click*
/// is delivered as a non-client message and swallowed — the button never
/// sees it. The shell's own flush-to-the-edge window controls (see
/// styles.css `#window-controls`) sit exactly under that strip.
///
/// Fix (all in the parent-window subclass):
///
/// - **Hide the drag-resize child while maximized, show it when
///   restored.** A hidden window takes no part in hit testing at all, so
///   the strip behaves identically to every row below it: the hit test
///   falls straight to the WebView2, and hover + click both work. When
///   restored, showing the child restores edge-resizing as tauri
///   designed it. The initial state is applied directly in
///   `patch_maximized_hit_test` (covering the startup-maximized ordering
///   where no WM_SIZE ever follows the attach), and kept in sync on every
///   later WM_SIZE.
/// - **Convert the parent's own border hits to HTCLIENT while
///   maximized** (a maximized window can't be resized, and the resize
///   cursor is exactly the complaint). On restore this passes through
///   untouched, so edge-resizing still works.
///
/// The maximized state is re-checked on every message, so
/// maximize/restore transitions need no bookkeeping beyond the WM_SIZE
/// show/hide above.
pub fn patch_maximized_hit_test(win: &tauri::WebviewWindow) {
    let Ok(hwnd) = win.hwnd() else {
        eprintln!("[dsh-desktop] window_proc: hwnd() failed");
        return;
    };

    unsafe {
        // The drag-resize child may legitimately be absent (e.g. if the
        // window were ever configured non-resizable) — FindWindowExW
        // errors out then, and the subclass just no-ops its child logic.
        let child = FindWindowExW(Some(hwnd), None, TAURI_DRAG_RESIZE_CLASS, TAURI_DRAG_RESIZE_NAME)
            .unwrap_or_default();
        let _ = SetWindowSubclass(hwnd, Some(parent_proc), SUBCLASS_ID_PARENT, child.0 as usize);

        // Startup-maximized ordering: the child was attached with the
        // maximized client size and no WM_SIZE will follow to hide it —
        // hide it now (and the WM_SIZE handler below keeps it in sync).
        if !child.is_invalid() && IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(child, SW_HIDE);
        }
    }
}

/// Parent-window subclass. Runs before tauri/tao's own window proc in the
/// subclass chain (SetWindowSubclass calls the last-installed proc first),
/// so this only ever adjusts behavior — never re-implements it.
unsafe extern "system" fn parent_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    data: usize,
) -> LRESULT {
    let child = HWND(data as _);

    match msg {
        WM_SIZE => {
            // The drag-resize child is only needed while the window can
            // be edge-resized. Keep it hidden while maximized so it
            // can't intercept hover/click at the screen's top strip; show
            // it again on restore so tauri's own WM_SIZE handler (which
            // runs after this one and re-sizes + re-regions the child)
            // brings edge-resizing back.
            if !child.is_invalid() {
                if IsZoomed(hwnd).as_bool() {
                    let _ = ShowWindow(child, SW_HIDE);
                } else {
                    let _ = ShowWindow(child, SW_SHOW);
                }
            }
        }
        WM_NCHITTEST => {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            if IsZoomed(hwnd).as_bool() {
                // A maximized window can't be resized, so a border hit is
                // useless — and at the top edge it's actively wrong
                // (resize cursor, and mouse messages that would land in
                // the non-client area instead of the page). Report the
                // client area; the WebView2 child under the cursor then
                // gets the mouse exactly like any other client point.
                let hit = result.0 as u32;
                if matches!(
                    hit,
                    HTTOP
                        | HTTOPLEFT
                        | HTTOPRIGHT
                        | HTLEFT
                        | HTRIGHT
                        | HTBOTTOM
                        | HTBOTTOMLEFT
                        | HTBOTTOMRIGHT
                ) {
                    return LRESULT(HTCLIENT as _);
                }
            }
            return result;
        }
        _ => {}
    }

    DefSubclassProc(hwnd, msg, wparam, lparam)
}
