//! Native menu and tray for the shell.
//!
//! Menu/tray item ids are handled in `lib.rs` (`handle_menu_action`) so the
//! actions can reach the shared app state; `build_tray` takes the handler as a
//! callback to avoid a circular dependency.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "macos")]
use tauri::menu::Submenu;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::i18n;

pub const MENU_OPEN_BROWSER: &str = "open_browser";
pub const MENU_RESTART: &str = "restart";
pub const MENU_OPEN_DATA_DIR: &str = "open_data_dir";
pub const MENU_SHOW_WINDOW: &str = "show_window";
pub const MENU_TOGGLE_AUTOSTART: &str = "toggle_autostart";
pub const MENU_QUIT: &str = "quit";

/// macOS-only — see the `set_menu()` callsite in lib.rs for why.
#[cfg(target_os = "macos")]
pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let lang = i18n::detect();
    // The first submenu becomes the macOS application menu (its name is
    // replaced by the app name). It must carry the standard roles — the
    // Quit role in particular is what binds ⌘Q (key equivalent "q" + the
    // terminate: selector); without it Cmd+Q is bound to nothing and the
    // shortcut silently does nothing. Quitting then funnels into Tauri's
    // ExitRequested flow, where lib.rs tears the server down.
    let app_menu = Submenu::with_items(
        app,
        "DeepSeek Harness",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let open_browser = MenuItem::with_id(app, MENU_OPEN_BROWSER, i18n::tr(lang, "在浏览器中打开", "Open in Browser"), true, None::<&str>)?;
    let restart = MenuItem::with_id(app, MENU_RESTART, i18n::tr(lang, "重启服务", "Restart Service"), true, None::<&str>)?;
    let open_data_dir = MenuItem::with_id(
        app,
        MENU_OPEN_DATA_DIR,
        i18n::tr(lang, "打开数据目录 (~/.dsh)", "Open Data Folder (~/.dsh)"),
        true,
        None::<&str>,
    )?;
    // The custom "退出" item is gone: the app menu's standard Quit role
    // covers quitting (with the ⌘Q shortcut), and tearing the server down
    // happens once in lib.rs's ExitRequested handler for every quit path.
    let file = Submenu::with_items(app, i18n::tr(lang, "文件", "File"), true, &[&open_browser, &restart, &open_data_dir])?;
    // macOS 上 Cmd+C / Cmd+V / Cmd+X / Cmd+A 等快捷键必须由菜单中的标准
    // "编辑"项提供（通过 responder chain 分发到 WebView），缺少它们会导致
    // 剪切/复制/粘贴失效。PredefinedMenuItem 的角色项会自动带上正确的
    // 快捷键与选择器，无需手动注册处理逻辑。它们自身的文本由 AppKit 按系统
    // 语言自动本地化，这里不需要（也不能）手动翻译。
    let edit = Submenu::with_items(
        app,
        i18n::tr(lang, "编辑", "Edit"),
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    // Standard Window roles give back ⌘M (minimize) and ⌘W (close window —
    // which this app's tray-resident design turns into hide). Without them
    // those shortcuts are dead for the same reason ⌘Q was.
    let window = Submenu::with_items(
        app,
        i18n::tr(lang, "窗口", "Window"),
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Ok(Menu::with_items(app, &[&app_menu, &file, &edit, &window])?)
}

/// Builds the tray icon/menu and returns the autostart toggle's
/// `CheckMenuItem` handle so the caller can keep its checked state in sync
/// after every toggle (see `MENU_TOGGLE_AUTOSTART` in lib.rs's
/// `handle_menu_action` — clicking a `CheckMenuItem` does not flip its own
/// visual state automatically).
pub fn build_tray(
    app: &AppHandle,
    on_action: impl Fn(&AppHandle, &str) + Send + Sync + 'static,
) -> tauri::Result<CheckMenuItem<Wry>> {
    let lang = i18n::detect();
    let show = MenuItem::with_id(app, MENU_SHOW_WINDOW, i18n::tr(lang, "显示窗口", "Show Window"), true, None::<&str>)?;
    let open_browser = MenuItem::with_id(app, MENU_OPEN_BROWSER, i18n::tr(lang, "在浏览器中打开", "Open in Browser"), true, None::<&str>)?;
    let restart = MenuItem::with_id(app, MENU_RESTART, i18n::tr(lang, "重启服务", "Restart Service"), true, None::<&str>)?;
    // On Windows/Linux this tray is the *only* menu (see the `set_menu()`
    // callsite in lib.rs) — include everything the removed window menu
    // offered, not just what macOS's menu bar leaves uncovered.
    let open_data_dir = MenuItem::with_id(
        app,
        MENU_OPEN_DATA_DIR,
        i18n::tr(lang, "打开数据目录 (~/.dsh)", "Open Data Folder (~/.dsh)"),
        true,
        None::<&str>,
    )?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        MENU_TOGGLE_AUTOSTART,
        i18n::tr(lang, "开机自动启动", "Launch at Startup"),
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, MENU_QUIT, i18n::tr(lang, "退出", "Quit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &open_browser, &restart, &open_data_dir, &autostart, &quit])?;

    // Shared between on_menu_event and on_tray_icon_event below (a left
    // click routes through the same MENU_SHOW_WINDOW id/handler as the
    // "显示窗口" item, so both paths stay in sync by construction).
    let on_action = Arc::new(on_action);
    let on_action_click = on_action.clone();

    let mut builder = TrayIconBuilder::with_id(crate::server::TRAY_ID)
        .tooltip("DeepSeek Harness")
        .menu(&menu)
        // Left click shows the window directly; only right click opens this
        // menu (the platform default once this is off).
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| on_action(app, event.id.as_ref()))
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                on_action_click(tray.app_handle(), MENU_SHOW_WINDOW);
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(autostart)
}
