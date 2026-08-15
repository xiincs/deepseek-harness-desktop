// Container page for dsh-desktop: hosts the harness UI in an <iframe> and
// renders the native file/git panel beside it. Talks to the Rust shell
// through Tauri IPC. The harness content inside the iframe never gets IPC
// access — window.__TAURI__ is injected into this top-level document only,
// and browsers don't propagate it into a cross-origin nested iframe.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const els = {
  starting: document.getElementById("state-starting"),
  error: document.getElementById("state-error"),
  harnessFrame: document.getElementById("harness-frame"),
  startingDetail: document.getElementById("starting-detail"),
  errorMessage: document.getElementById("error-message"),
  logBox: document.getElementById("log-box"),
  logBoxStarting: document.getElementById("log-box-starting"),
  btnLogsStarting: document.getElementById("btn-logs-starting"),
  btnRetry: document.getElementById("btn-retry"),
  btnRestart: document.getElementById("btn-restart"),
  btnLogs: document.getElementById("btn-logs"),
  btnOpenBrowser: document.getElementById("btn-open-browser"),
  footer: document.getElementById("footer"),
  updateBanner: document.getElementById("update-banner"),
  updateText: document.getElementById("update-text"),
  btnUpdateInstall: document.getElementById("btn-update-install"),
  btnUpdateDismiss: document.getElementById("btn-update-dismiss"),
  providerTip: document.getElementById("provider-tip"),
  btnProviderTipDismiss: document.getElementById("btn-provider-tip-dismiss"),
  panel: document.getElementById("panel"),
  panelWorkspaceSelect: document.getElementById("panel-workspace-select"),
  panelTree: document.getElementById("panel-tree"),
  btnPanelRefresh: document.getElementById("btn-panel-refresh"),
  resizePanelContent: document.getElementById("resize-panel-content"),
  panelCards: document.getElementById("panel-cards"),
  resizePanelCards: document.getElementById("resize-panel-cards"),
  btnToolbarFiles: document.getElementById("btn-toolbar-files"),
  btnToolbarTerminal: document.getElementById("btn-toolbar-terminal"),
  cardTerminal: document.getElementById("card-terminal"),
  terminalContainer: document.getElementById("terminal-container"),
  btnTerminalRestart: document.getElementById("btn-terminal-restart"),
  btnTerminalClose: document.getElementById("btn-terminal-close"),
  toolbar: document.getElementById("toolbar"),
  windowControls: document.getElementById("window-controls"),
  btnWinMinimize: document.getElementById("btn-win-minimize"),
  btnWinMaximize: document.getElementById("btn-win-maximize"),
  btnWinClose: document.getElementById("btn-win-close"),
  btnFilesCollapse: document.getElementById("btn-files-collapse"),
  btnAppMenu: document.getElementById("btn-app-menu"),
  appMenu: document.getElementById("app-menu"),
  cardFiles: document.getElementById("card-files"),
  cardFile: document.getElementById("card-file"),
  panelPreviewTitle: document.getElementById("panel-preview-title"),
  panelPreviewDirtyDot: document.getElementById("panel-preview-dirty-dot"),
  panelPreviewBody: document.getElementById("panel-preview-body"),
  btnPreviewClose: document.getElementById("btn-preview-close"),
  btnPreviewSave: document.getElementById("btn-preview-save"),
  btnPreviewRevert: document.getElementById("btn-preview-revert"),
};

// Shown once (best-effort) during the first-ever boot wait, so new users
// discover the existing Settings → 模型 → 添加提供方 flow without us having
// to touch the harness page itself (it's iframed content with zero IPC
// access — see lib.rs).
const PROVIDER_TIP_DISMISSED_KEY = "dsh-desktop-provider-tip-dismissed";

function initProviderTip() {
  if (localStorage.getItem(PROVIDER_TIP_DISMISSED_KEY)) return;
  els.providerTip.classList.remove("hidden");
  els.btnProviderTipDismiss.addEventListener("click", () => {
    localStorage.setItem(PROVIDER_TIP_DISMISSED_KEY, "1");
    els.providerTip.classList.add("hidden");
  });
}

let logsVisible = false;
let logsStartingVisible = false;

function show(id) {
  for (const key of ["starting", "error"]) {
    els[key].classList.toggle("hidden", key !== id);
  }
  els.harnessFrame.classList.toggle("hidden", id !== "running");
}

async function loadLogsInto(box) {
  try {
    const lines = await invoke("get_log_tail", { n: 200 });
    box.textContent = lines.join("\n");
  } catch (err) {
    box.textContent = `无法读取日志: ${err}`;
  }
}

function toggleLogs() {
  logsVisible = !logsVisible;
  els.logBox.classList.toggle("hidden", !logsVisible);
  els.btnLogs.textContent = logsVisible ? "隐藏日志" : "查看日志";
  if (logsVisible) loadLogsInto(els.logBox);
}

function toggleLogsStarting() {
  logsStartingVisible = !logsStartingVisible;
  els.logBoxStarting.classList.toggle("hidden", !logsStartingVisible);
  els.btnLogsStarting.textContent = logsStartingVisible ? "隐藏日志" : "查看日志";
  if (logsStartingVisible) loadLogsInto(els.logBoxStarting);
}

function render(status) {
  switch (status.state) {
    case "running":
      show("running");
      els.harnessFrame.src = status.url;
      refreshPanel();
      break;
    case "starting":
    case "idle":
      show("starting");
      els.startingDetail.textContent = status.detail || "准备本地服务";
      break;
    case "stopped":
      show("error");
      els.harnessFrame.src = "about:blank";
      els.errorMessage.textContent =
        `服务已停止（exit ${status.code ?? "?"}）。` +
        (status.message ? `\n${status.message}` : "");
      break;
    case "error":
      show("error");
      els.harnessFrame.src = "about:blank";
      els.errorMessage.textContent = status.message || "未知错误";
      break;
    default:
      show("starting");
  }
}

async function refresh() {
  try {
    const status = await invoke("get_status");
    render(status);
  } catch (err) {
    show("error");
    els.errorMessage.textContent = `无法获取状态: ${err}`;
  }
}

// ── app menu (hamburger, left of the drag region) ──────────────────────
//
// Fronts the same five actions menu.rs's tray menu already offers
// (MENU_OPEN_BROWSER/RESTART/OPEN_DATA_DIR/TOGGLE_AUTOSTART/QUIT) — those
// are otherwise only reachable via the tray icon on Windows/Linux, since
// decorations:false leaves no native menu bar for them to live in (see
// menu.rs's set_menu() macOS-only gate). trigger_menu_action forwards the
// clicked id straight to lib.rs's handle_menu_action, the same dispatcher
// the tray's own on_menu_event already calls — no action logic duplicated
// here, this is only presentation. Not wired up at all on macOS (see
// init()'s own !IS_MACOS guard below) — the native menu bar already covers
// this, and the hamburger button itself is hidden there (styles.css
// body.platform-decorated).

function isAppMenuOpen() {
  return !els.appMenu.classList.contains("hidden");
}

async function openAppMenu() {
  // Re-read on every open rather than caching: the tray's own "开机自动
  //启动" checkbox can be toggled independently of this menu (or the OS
  // setting changed outside the app entirely), so a stale cached value
  // could show a checkmark that no longer matches reality.
  let enabled = false;
  try {
    enabled = await invoke("get_autostart_enabled");
  } catch {
    /* leave unchecked rather than block opening the menu over this */
  }
  els.appMenu.querySelector(".app-menu-check").classList.toggle("hidden", !enabled);
  els.appMenu.classList.remove("hidden");
  els.btnAppMenu.setAttribute("aria-expanded", "true");
}

function closeAppMenu() {
  els.appMenu.classList.add("hidden");
  els.btnAppMenu.setAttribute("aria-expanded", "false");
}

function initAppMenu() {
  els.btnAppMenu.addEventListener("click", (event) => {
    event.stopPropagation();
    if (isAppMenuOpen()) {
      closeAppMenu();
    } else {
      openAppMenu();
    }
  });

  for (const item of els.appMenu.querySelectorAll(".app-menu-item")) {
    item.addEventListener("click", () => {
      const id = item.dataset.menuId;
      closeAppMenu();
      invoke("trigger_menu_action", { id });
    });
  }

  // Outside click / Escape — the two standard ways a dropdown expects to
  // be dismissed without acting on anything.
  document.addEventListener("click", (event) => {
    if (isAppMenuOpen() && !els.appMenu.contains(event.target)) closeAppMenu();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && isAppMenuOpen()) closeAppMenu();
  });
}

// ── window chrome (per-platform) ────────────────────────────────────────
//
// decorations:false in tauri.conf.json removes the OS title bar on every
// platform. On Windows/Linux the shell keeps that frameless look: #toolbar
// carries data-tauri-drag-region (see index.html) so its own empty space
// moves the window, and the three custom buttons below stand in for the
// native minimize/maximize/close. On macOS lib.rs re-enables the native
// title bar (real traffic lights on the left, native drag), so the custom
// replacements are hidden and the toolbar stops acting as a drag region —
// see initWindowChrome() below.
// lib.rs's on_window_event CloseRequested handler (hide-to-tray) is keyed
// off the window-close request itself, not off which button drew it — so
// appWindow.close() below re-enters that exact same Rust-side path with
// nothing to change there.

const appWindow = getCurrentWindow();

// Mirrors lib.rs's compile-time `#[cfg(target_os = "macos")]` decorations
// split. The UA is deterministic at load time, unlike querying isDecorated()
// which could race with the Rust-side set_decorations(true) during setup.
const IS_MACOS = navigator.userAgent.includes("Macintosh");

function initWindowChrome() {
  // #window-controls is hidden by default in index.html so it can never
  // paint before this decision runs — on macOS the native traffic lights
  // take over, and a cold-start frame with BOTH the native lights and the
  // custom buttons would otherwise flash. Only the frameless Windows/Linux
  // chrome reveals the custom controls.
  if (IS_MACOS) {
    // Native title bar takes over window dragging and min/max/close — the
    // custom replacements would only duplicate it (and its drag region would
    // fight the native double-click-to-zoom on the title bar).
    els.toolbar.removeAttribute("data-tauri-drag-region");
    // Left-align the remaining toolbar actions like a standard macOS toolbar
    // (see styles.css body.platform-decorated).
    document.body.classList.add("platform-decorated");
  } else {
    els.windowControls.classList.remove("hidden");
  }
}

const ICON_MAXIMIZE =
  '<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor"/></svg>';
const ICON_RESTORE =
  '<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><rect x="3" y="0.5" width="6.5" height="6.5" fill="none" stroke="currentColor"/><rect x="0.5" y="3" width="6.5" height="6.5" fill="none" stroke="currentColor"/></svg>';

async function syncMaximizeIcon() {
  const maximized = await appWindow.isMaximized();
  els.btnWinMaximize.innerHTML = maximized ? ICON_RESTORE : ICON_MAXIMIZE;
  els.btnWinMaximize.title = maximized ? "还原" : "最大化";
}

function initWindowControls() {
  els.btnWinMinimize.addEventListener("click", () => appWindow.minimize());
  els.btnWinMaximize.addEventListener("click", () => appWindow.toggleMaximize());
  els.btnWinClose.addEventListener("click", () => appWindow.close());
  syncMaximizeIcon();
  appWindow.onResized(syncMaximizeIcon);
}

// ── resizable dock width ────────────────────────────────────────────────
//
// One draggable, persisted split: the dock (#panel, now right-docked)
// against #content (the harness iframe). Applies as an inline style (see
// applyPanelWidth) rather than living in styles.css, since a CSS width
// can't be end-user-adjustable without JS setting it somewhere; the
// stylesheet keeps a single fallback default for the instant before this
// script runs. See "resizable card split" below for the second, vertical
// drag handle this same pattern extends to, between the two dock cards.

const PANEL_WIDTH_KEY = "dsh-desktop-panel-width";
const DEFAULT_PANEL_WIDTH = 380;
// Never shrinks the dock below this — small enough to still show a few
// characters of a filename or a code line, too small to accidentally
// collapse it to nothing mid-drag.
const MIN_PANEL_WIDTH = 240;
// The dock may never claim more than this fraction of the window — #content
// (the harness) staying visibly present matters more than an oversized dock.
const MAX_PANEL_WIDTH_RATIO = 0.7;

function loadStoredPixels(key, fallback) {
  const n = parseInt(localStorage.getItem(key), 10);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

let panelWidth = loadStoredPixels(PANEL_WIDTH_KEY, DEFAULT_PANEL_WIDTH);

function applyPanelWidth() {
  els.panel.style.width = `${panelWidth}px`;
}

function initResizeHandle() {
  els.resizePanelContent.addEventListener("mousedown", (downEvent) => {
    downEvent.preventDefault();
    els.resizePanelContent.classList.add("dragging");
    document.body.classList.add("resizing");

    // #panel is now the right-docked element, so its width is measured
    // from the window's right edge inward, not directly from clientX (that
    // reasoning only held when #panel started flush against the left edge).
    const onMouseMove = (moveEvent) => {
      const candidate = window.innerWidth - moveEvent.clientX;
      const max = window.innerWidth * MAX_PANEL_WIDTH_RATIO;
      panelWidth = Math.max(MIN_PANEL_WIDTH, Math.min(max, candidate));
      applyPanelWidth();
    };
    const onMouseUp = () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      els.resizePanelContent.classList.remove("dragging");
      document.body.classList.remove("resizing");
      localStorage.setItem(PANEL_WIDTH_KEY, String(Math.round(panelWidth)));
    };
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  });
}

// ── resizable card split ────────────────────────────────────────────────
//
// A second, vertical drag handle between #card-files and #card-file —
// only meaningful while both are in their normal (non-collapsed, open)
// state; syncCardResizeHandleVisibility hides it otherwise, and the
// #card-files.card-collapsed ~ #card-file CSS rule already gives File the
// full height on its own once Files collapses, independent of whatever
// height was last dragged here.
//
// Unset (null) until the user actually drags: #card-file's CSS
// max-height:60% default keeps applying until then, same as
// DEFAULT_PANEL_WIDTH's role above but via "no override yet" rather than a
// numeric default, since unlike the dock's width this one has a perfectly
// good zero-JS fallback already in the stylesheet.
const CARD_FILE_HEIGHT_KEY = "dsh-desktop-card-file-height";
let cardFileHeight = loadStoredPixels(CARD_FILE_HEIGHT_KEY, null);

// Matches #card-files' own CSS min-height — the floor this drag leaves it,
// so dragging past that point simply stops growing #card-file further
// rather than the two fighting over the same pixels.
const MIN_FILES_HEIGHT = 120;
const MIN_CARD_FILE_HEIGHT = 100;

function applyCardFileHeight() {
  // Collapsed Files already hands File 100% via CSS — an inline style here
  // would (inline always outranks a class selector) override that and pin
  // File back to its last dragged height instead, so this stays cleared
  // for exactly as long as Files is collapsed.
  const collapsed = els.cardFiles.classList.contains("card-collapsed");
  if (collapsed || cardFileHeight === null) {
    els.cardFile.style.flexBasis = "";
    els.cardFile.style.maxHeight = "";
  } else {
    els.cardFile.style.flexBasis = `${cardFileHeight}px`;
    els.cardFile.style.maxHeight = "none";
  }
}

// The handle only makes sense — and is only shown — while there's an
// actual split to drag: Files expanded and a file open in the card below.
function syncCardResizeHandleVisibility() {
  const visible = !els.cardFiles.classList.contains("card-collapsed") && !els.cardFile.classList.contains("hidden");
  els.resizePanelCards.classList.toggle("hidden", !visible);
}

function initCardsResizeHandle() {
  els.resizePanelCards.addEventListener("mousedown", (downEvent) => {
    downEvent.preventDefault();
    els.resizePanelCards.classList.add("dragging");
    document.body.classList.add("resizing-rows");

    const startY = downEvent.clientY;
    const startHeight = els.cardFile.getBoundingClientRect().height;

    // The handle sits between Files (above) and File (below): dragging up
    // (negative delta) shrinks Files and should grow File, hence the sign
    // flip versus a naive "add the delta" — mirrors initResizeHandle's own
    // clientX-from-the-right-edge inversion above for the same reason.
    const onMouseMove = (moveEvent) => {
      const delta = moveEvent.clientY - startY;
      const candidate = startHeight - delta;
      const max = Math.max(MIN_CARD_FILE_HEIGHT, els.panelCards.clientHeight - MIN_FILES_HEIGHT);
      cardFileHeight = Math.max(MIN_CARD_FILE_HEIGHT, Math.min(max, candidate));
      applyCardFileHeight();
    };
    const onMouseUp = () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      els.resizePanelCards.classList.remove("dragging");
      document.body.classList.remove("resizing-rows");
      localStorage.setItem(CARD_FILE_HEIGHT_KEY, String(Math.round(cardFileHeight)));
    };
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  });
}

// ── dock open/closed ────────────────────────────────────────────────────
//
// Closed on every launch (not persisted) — the toolbar's 文件/终端 buttons
// are each a deliberate opt-in per session, not state to restore. Only the
// width (once opened) is remembered, via panelWidth/PANEL_WIDTH_KEY above.
//
// #panel itself is shared by both: it stays visible as long as *either*
// toggle is active, so closing Files while Terminal is open collapses only
// the Files card, not the dock the Terminal card is still sitting in.

function syncPanelVisibility() {
  const open = els.btnToolbarFiles.classList.contains("active") || els.btnToolbarTerminal.classList.contains("active");
  els.panel.classList.toggle("hidden", !open);
  els.resizePanelContent.classList.toggle("hidden", !open);
}

function setDockOpen(open) {
  els.btnToolbarFiles.classList.toggle("active", open);
  syncPanelVisibility();
  if (open) refreshPanel();
}

function toggleDock() {
  setDockOpen(!els.btnToolbarFiles.classList.contains("active"));
}

// ── terminal ─────────────────────────────────────────────────────────────
//
// One singleton xterm.js instance for the card's whole lifetime — closing
// the card hides it (and, via btn-terminal-close, kills the backing shell),
// but the Terminal object itself and its scrollback are kept around so
// reopening doesn't pay xterm's own init cost again. The Rust-side session
// (terminal.rs) has the same "hide keeps it alive" split: only an explicit
// close/restart, or the app actually quitting, kills the shell process.

let xterm = null;
let fitAddon = null;
// Tracks whether *this renderer* believes a backend shell is alive — reset
// on close/restart and on the backend's own "terminal-exit" event (e.g. the
// user typed `exit`), so the next open respawns instead of writing into a
// dead pty.
let terminalSpawned = false;

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// No-op while the card is hidden (display:none reports a zero-size box —
// fitting against that would shrink the pty to 0 cols/rows) or before
// xterm has ever been opened.
function fitTerminal() {
  if (!xterm || !fitAddon || els.cardTerminal.classList.contains("hidden")) return;
  try {
    fitAddon.fit();
  } catch {
    return;
  }
  if (terminalSpawned) {
    invoke("terminal_resize", { cols: xterm.cols, rows: xterm.rows }).catch(() => {});
  }
}

async function ensureTerminal() {
  if (!xterm) {
    const style = getComputedStyle(document.documentElement);
    xterm = new window.XTerm.Terminal({
      fontFamily: '"SF Mono", "JetBrains Mono", "Fira Code", Consolas, Menlo, monospace',
      fontSize: 12.5,
      cursorBlink: true,
      theme: {
        background: style.getPropertyValue("--terminal-bg").trim(),
        foreground: style.getPropertyValue("--terminal-fg").trim(),
      },
    });
    fitAddon = new window.XTerm.FitAddon();
    xterm.loadAddon(fitAddon);
    xterm.open(els.terminalContainer);
    // Raw keystrokes/paste go straight to the pty — the shell on the other
    // end owns line editing (backspace, history, completion), not us.
    xterm.onData((data) => {
      invoke("terminal_write", { data }).catch(() => {});
    });
    new ResizeObserver(fitTerminal).observe(els.terminalContainer);
  }
  if (terminalSpawned) return;
  fitTerminal();
  xterm.clear();
  try {
    await invoke("terminal_spawn", { cols: xterm.cols || 80, rows: xterm.rows || 24 });
    terminalSpawned = true;
  } catch (err) {
    xterm.writeln(`\r\n\x1b[31m终端启动失败: ${err}\x1b[0m`);
  }
}

async function setTerminalOpen(open) {
  els.btnToolbarTerminal.classList.toggle("active", open);
  els.cardTerminal.classList.toggle("hidden", !open);
  syncPanelVisibility();
  if (open) {
    await ensureTerminal();
    // Belt-and-suspenders: the geometry read inside ensureTerminal's own
    // fitTerminal() call already forces a synchronous layout, but this
    // re-fits once more after the browser's next paint in case some part
    // of the just-revealed layout (the dock, this card) wasn't settled yet.
    requestAnimationFrame(fitTerminal);
  }
}

function toggleTerminal() {
  setTerminalOpen(els.cardTerminal.classList.contains("hidden"));
}

// ── file/git panel ───────────────────────────────────────────────────────

const GIT_STATUS_CLASS = {
  modified: "git-modified",
  added: "git-added",
  deleted: "git-deleted",
  untracked: "git-untracked",
};

// One-letter badge text, paired with GIT_STATUS_CLASS for color (see
// renderTreeNode) — VS Code's own convention, and a second, non-color-only
// encoding of the same status the row's text tint already carries.
const GIT_STATUS_LETTER = {
  modified: "M",
  added: "A",
  deleted: "D",
  untracked: "U",
};

// `name` is always a call-site literal naming a <symbol> in the sprite
// (index.html), never derived from file/path data — safe to build via
// innerHTML since nothing here is untrusted input.
//
// `cls` must land on the <svg> itself, not a wrapping element: an <svg>
// with no width/height/viewBox of its own falls back to the UA default
// intrinsic size (300×150) regardless of how a parent is sized, since
// sizing a container doesn't scale unsized replaced content inside it. The
// static icons elsewhere in index.html already put their class directly on
// <svg> for this reason — this mirrors that instead of introducing a second
// (broken) pattern.
function iconEl(name, cls) {
  const wrap = document.createElement("span");
  wrap.innerHTML = `<svg class="${cls}"><use href="#icon-${name}"></use></svg>`;
  return wrap.firstElementChild;
}

// "" is the sentinel for "auto-follow" in the <select> — never a real
// filesystem path, so it can't collide with an actual workspace's value.
const AUTO_OPTION_VALUE = "";
const LOCKED_WORKSPACE_KEY = "dsh-desktop-locked-workspace";

// null = auto-follow (get_active_workspace's live, best-effort inference —
// see its Rust-side doc comment for why that's a real ceiling, not a
// shortcut); a string is the user's own pick from the panel's picker,
// passed straight through as get_workspace_tree/get_git_status's
// overridePath. Persisted so an explicit choice survives a restart, the
// same pattern PROVIDER_TIP_DISMISSED_KEY already uses for a one-time
// user decision.
let lockedWorkspace = localStorage.getItem(LOCKED_WORKSPACE_KEY) || null;

// Paths of directories the user collapsed. The tree is rebuilt from scratch
// on every poll (see refreshTreeAndGitStatus's replaceChildren), so without
// this the collapsed state would be lost and folders would re-expand within
// PANEL_POLL_MS of being collapsed.
//
// TreeEntry.path is workspace-RELATIVE (see panel.rs), so these keys are
// only meaningful for the workspace they were collapsed in — switching
// workspaces must clear the set, or workspace B would silently inherit A's
// collapse state for any same-relative-path directory (the same reason the
// workspace-switch handler closes the open preview).
const collapsedDirs = new Set();

// Which workspace the last-rendered tree belonged to: lockedWorkspace when
// pinned, else the auto-follow resolution. A change clears collapsedDirs.
let renderedWorkspaceKey = null;

function applyWorkspaceChange(workspaceKey) {
  if (workspaceKey === null || workspaceKey === renderedWorkspaceKey) return;
  renderedWorkspaceKey = workspaceKey;
  collapsedDirs.clear();
}

function renderTreeNode(entry, gitMap, container) {
  const row = document.createElement("div");
  row.className = "tree-row" + (entry.isDir ? " tree-dir" : " tree-file");
  const status = gitMap.get(entry.path);
  if (status && GIT_STATUS_CLASS[status]) row.classList.add(GIT_STATUS_CLASS[status]);
  if (!entry.isDir && entry.path === currentPreviewPath) row.classList.add("tree-row-selected");

  const hasChildren = entry.isDir && entry.children;
  // Directories get a real disclosure caret (rotated via .tree-expanded,
  // see styles.css); files get an equal-width blank spacer — a genuinely
  // empty element, not a caret icon in a differently-colored class, so
  // there's no icon to inherit stray color from — so a file's own icon
  // still lines up in the same column as its siblings' carets rather than
  // sitting one indent level to the left of them.
  if (hasChildren) {
    row.appendChild(iconEl("chevron-right", "tree-caret"));
    // Default is expanded; collapsed dirs render collapsed from the start so
    // the user's choice survives the periodic full rebuild.
    row.classList.toggle("tree-expanded", !collapsedDirs.has(entry.path));
  } else {
    const spacer = document.createElement("span");
    spacer.className = "tree-caret-spacer";
    row.appendChild(spacer);
  }
  row.appendChild(iconEl(entry.isDir ? "folder" : "file", "tree-icon"));

  const label = document.createElement("span");
  label.className = "tree-label";
  label.textContent = entry.name;
  row.appendChild(label);

  if (status && GIT_STATUS_LETTER[status]) {
    const badge = document.createElement("span");
    badge.className = "git-badge";
    badge.textContent = GIT_STATUS_LETTER[status];
    row.appendChild(badge);
  }

  container.appendChild(row);

  if (hasChildren) {
    const childWrap = document.createElement("div");
    childWrap.className = "tree-children";
    if (collapsedDirs.has(entry.path)) childWrap.classList.add("collapsed");
    container.appendChild(childWrap);
    row.addEventListener("click", () => {
      const collapsed = childWrap.classList.toggle("collapsed");
      row.classList.toggle("tree-expanded", !collapsed);
      if (collapsed) collapsedDirs.add(entry.path);
      else collapsedDirs.delete(entry.path);
    });
    for (const child of entry.children) {
      renderTreeNode(child, gitMap, childWrap);
    }
  } else if (!entry.isDir) {
    row.addEventListener("click", () => {
      if (currentPreviewPath === entry.path) {
        closePreview();
      } else {
        showPreview(entry.path);
      }
    });
  }
}

// ── file preview / edit (CodeMirror) ────────────────────────────────────
//
// Opening a file goes straight to an editable CodeMirror instance — no
// separate read-only "view" the user has to explicitly leave to edit. When
// the file has a git status worth comparing against (Modified/Deleted),
// @codemirror/merge's unifiedMergeView adds inline gutter decorations for
// the changed regions on top of that same editable pane (VS Code's own
// pattern: gutter markers are informational, not a second view to switch
// into), diffing client-side against the file's last-committed (HEAD)
// content — no more hand-rolled diff parsing on the Rust side.
//
// Two independent "did this change" baselines coexist deliberately:
//   1. git HEAD vs current content → the merge view's own gutter/inline
//      decorations. Unaffected by saving to disk (HEAD only moves on a
//      commit) — never needs refetching after a save.
//   2. last load-or-save vs the live, possibly-unsaved editor content →
//      this file's own dirty-dot/保存/还原 tracking, via currentSavedContent
//      below. Reset to "clean" on every successful save.
// "还原" only ever undoes (1) against baseline (2) — the current *editing
// session's* unsaved typing — never git's committed history. Conflating
// the two would make a UI button that reads as "undo my typing" silently
// capable of discarding a git-tracked change instead.

let currentPreviewPath = null;
let currentEditorView = null;
// The content as of the last successful load or save — see the baseline
// note above. `null` whenever no editor is mounted.
let currentSavedContent = null;
// Built once, lazily, and reused by every editor instance — colors are
// resolved via var(...) at paint time, so they already stay correct across
// a live prefers-color-scheme change without rebuilding this.
let cmBaseExtensions = null;

function buildCodeMirrorBaseExtensions() {
  if (cmBaseExtensions) return cmBaseExtensions;
  const CM = window.CM;
  const t = CM.tags;
  const highlightStyle = CM.HighlightStyle.define([
    { tag: t.comment, color: "var(--muted)", fontStyle: "italic" },
    { tag: [t.string, t.special(t.string)], color: "var(--success-text)" },
    { tag: [t.number, t.bool, t.null], color: "var(--danger)" },
    { tag: [t.keyword, t.controlKeyword, t.operatorKeyword, t.moduleKeyword], color: "var(--accent)" },
    { tag: [t.function(t.variableName), t.className, t.typeName], color: "var(--accent-tint-text)" },
    { tag: t.propertyName, color: "var(--text)" },
    { tag: t.punctuation, color: "var(--muted)" },
    { tag: t.tagName, color: "var(--accent)" },
    { tag: t.attributeName, color: "var(--accent-tint-text)" },
    { tag: t.invalid, color: "var(--danger)", textDecoration: "underline" },
  ]);

  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = CM.EditorView.theme(
    {
      "&": { height: "100%", fontSize: "11px", backgroundColor: "var(--card)", color: "var(--text)" },
      ".cm-content": {
        fontFamily: "'SF Mono','JetBrains Mono','Fira Code',Consolas,Menlo,monospace",
        caretColor: "var(--text)",
        padding: "8px 0",
      },
      ".cm-gutters": { backgroundColor: "var(--sidebar-bg)", color: "var(--muted)", border: "none" },
      ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--accent) 6%, transparent)" },
      ".cm-activeLineGutter": { backgroundColor: "color-mix(in srgb, var(--accent) 10%, transparent)" },
      "&.cm-focused .cm-cursor": { borderLeftColor: "var(--text)" },
      ".cm-scroller": { overflow: "auto" },
      // @codemirror/merge's own decoration classes — overridden to this
      // project's tokens rather than its default hardcoded rgba() colors
      // (same "no new palette" rule already applied to every other color
      // in this file).
      ".cm-changedLine": { backgroundColor: "color-mix(in srgb, var(--accent) 10%, transparent)" },
      ".cm-changedLineGutter": { backgroundColor: "color-mix(in srgb, var(--accent) 18%, transparent)" },
      ".cm-changedText": { backgroundColor: "color-mix(in srgb, var(--accent) 22%, transparent)" },
      ".cm-deletedChunk": { backgroundColor: "color-mix(in srgb, var(--danger) 6%, transparent)" },
      ".cm-deletedLine": { backgroundColor: "color-mix(in srgb, var(--danger) 10%, transparent)" },
      ".cm-deletedLineGutter": { backgroundColor: "color-mix(in srgb, var(--danger) 18%, transparent)" },
      ".cm-deletedText": { background: "none", textDecoration: "line-through", color: "var(--danger)" },
      ".cm-insertedLine": { backgroundColor: "color-mix(in srgb, var(--success-text) 12%, transparent)" },
    },
    { dark },
  );

  cmBaseExtensions = [CM.syntaxHighlighting(highlightStyle), theme];
  return cmBaseExtensions;
}

// Extension → CodeMirror language extension, for the fixed package set
// bundled in vendor/codemirror. An unmapped extension (or none) just opens
// unhighlighted plain text rather than failing.
function languageExtensionForPath(path) {
  const CM = window.CM;
  if (!CM) return null;
  const L = CM.languages;
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "js": case "mjs": case "cjs": return L.javascript();
    case "jsx": return L.javascript({ jsx: true });
    case "ts": return L.javascript({ typescript: true });
    case "tsx": return L.javascript({ jsx: true, typescript: true });
    case "rs": return L.rust();
    case "py": return L.python();
    case "json": return L.json();
    case "css": return L.css();
    case "html": case "htm": return L.html();
    case "md": return L.markdown();
    case "yaml": case "yml": return L.yaml();
    case "sql": return L.sql();
    case "java": return L.java();
    case "go": return L.go();
    case "c": case "h": case "cpp": case "cc": case "hpp": case "cxx": return L.cpp();
    default: return null;
  }
}

function isDirty() {
  return currentEditorView !== null && currentSavedContent !== null && currentEditorView.state.doc.toString() !== currentSavedContent;
}

function setDirty(dirty) {
  els.panelPreviewDirtyDot.classList.toggle("hidden", !dirty);
  els.btnPreviewSave.classList.toggle("hidden", !dirty);
  els.btnPreviewRevert.classList.toggle("hidden", !dirty);
}

// True if it's safe to proceed with whatever's about to replace or close
// the current preview: nothing open, nothing unsaved, or the user
// explicitly confirmed discarding it. Never mutates state itself — the
// caller that gets `true` back is the one actually tearing down or
// replacing the editor.
function confirmDiscardIfNeeded() {
  if (!isDirty()) return true;
  return confirm("有未保存的改动，确定要放弃吗？");
}

function destroyEditor() {
  if (currentEditorView) {
    currentEditorView.destroy();
    currentEditorView = null;
  }
  currentSavedContent = null;
  setDirty(false);
}

function contentTextOrNull(fileContent) {
  return fileContent && fileContent.kind === "text" ? fileContent.content : null;
}

// `preview` is a Rust EditablePreview: { current: FileContent | null,
// original: FileContent | null }. `current` is null only for a git-Deleted
// file (nothing left on disk); `original` is null unless there's a HEAD
// version worth diffing against (Modified/Deleted).
function mountEditor(path, preview) {
  const CM = window.CM;
  destroyEditor();
  els.panelPreviewBody.replaceChildren();

  if (preview.current === null) {
    const originalText = contentTextOrNull(preview.original);
    if (originalText === null) {
      els.panelPreviewBody.textContent = "无法读取此文件的历史内容";
      return;
    }
    const extensions = [CM.basicSetup, ...buildCodeMirrorBaseExtensions(), CM.EditorState.readOnly.of(true)];
    const lang = languageExtensionForPath(path);
    if (lang) extensions.push(lang);
    currentEditorView = new CM.EditorView({ doc: originalText, extensions, parent: els.panelPreviewBody });
    currentSavedContent = originalText;
    return;
  }

  const current = preview.current;
  if (current.kind === "binary") {
    els.panelPreviewBody.textContent = "二进制文件，无法预览";
    return;
  }
  if (current.kind === "tooLarge") {
    els.panelPreviewBody.textContent = `文件过大（${(current.bytes / 1024 / 1024).toFixed(1)} MB），未加载预览`;
    return;
  }
  if (current.kind === "error") {
    els.panelPreviewBody.textContent = current.message;
    return;
  }

  const originalText = contentTextOrNull(preview.original);
  const lang = languageExtensionForPath(path);
  const extensions = [
    CM.basicSetup,
    ...buildCodeMirrorBaseExtensions(),
    CM.keymap.of([
      CM.indentWithTab,
      { key: "Mod-s", preventDefault: true, run: () => (saveCurrentEdit(), true) },
    ]),
    CM.EditorView.updateListener.of((update) => {
      if (update.docChanged) setDirty(isDirty());
    }),
  ];
  if (lang) extensions.push(lang);
  // gutter markers only, no per-chunk accept/reject buttons — this is an
  // ordinary editable file with an informational "differs from HEAD"
  // signal, not a merge-conflict resolution UI.
  if (originalText !== null) extensions.push(CM.unifiedMergeView({ original: originalText, mergeControls: false }));

  currentEditorView = new CM.EditorView({ doc: current.content, extensions, parent: els.panelPreviewBody });
  currentSavedContent = current.content;
  setDirty(false);
}

async function showPreview(path) {
  if (!confirmDiscardIfNeeded()) return;
  currentPreviewPath = path;
  els.cardFile.classList.remove("hidden");
  syncCardResizeHandleVisibility();
  els.panelPreviewTitle.textContent = path;
  els.panelPreviewTitle.title = path;
  destroyEditor();
  els.panelPreviewBody.replaceChildren();
  const loading = document.createElement("p");
  loading.className = "muted panel-empty";
  loading.textContent = "加载中…";
  els.panelPreviewBody.appendChild(loading);

  try {
    const preview = await invoke("get_editable_preview", { path, overridePath: lockedWorkspace });
    // A slower load may resolve after the user already clicked a different
    // file (or closed the preview) — never let a stale response overwrite
    // whatever's actually being shown now.
    if (currentPreviewPath !== path) return;
    mountEditor(path, preview);
  } catch (err) {
    if (currentPreviewPath !== path) return;
    els.panelPreviewBody.textContent = `预览加载失败: ${err}`;
  }
}

// Re-fetches the open preview on the same cadence as the tree/git-status
// poll (called from refreshPanel) — the agent is very plausibly editing the
// exact file being previewed. Skipped entirely while there's an unsaved
// local edit: a background poll must never clobber that, and re-mounting a
// fresh editor on every tick would also reset cursor/scroll/undo history
// for no reason once the file has settled.
async function refreshCurrentPreview() {
  if (currentPreviewPath === null || isDirty()) return;
  const path = currentPreviewPath;
  try {
    const preview = await invoke("get_editable_preview", { path, overridePath: lockedWorkspace });
    if (currentPreviewPath !== path) return;
    // Skip the remount when the fetched text is identical to what's already
    // mounted — the common case on every 6s tick. mountEditor() always tears
    // down and recreates the CodeMirror instance, resetting cursor/scroll to
    // the start of the doc; without this check that fired every tick even
    // when nothing had changed, silently yanking the cursor back to
    // position 0 right as the user clicked in to place it, before typing
    // anything (isDirty() alone doesn't catch that moment — content is
    // still identical to currentSavedContent until the first keystroke).
    const freshText = contentTextOrNull(preview.current);
    if (freshText !== null && freshText === currentSavedContent) return;
    mountEditor(path, preview);
  } catch {
    /* leave whatever's already showing rather than blanking it over a transient poll failure */
  }
}

function closePreview() {
  if (!confirmDiscardIfNeeded()) return;
  currentPreviewPath = null;
  destroyEditor();
  els.cardFile.classList.add("hidden");
  els.panelPreviewBody.replaceChildren();
  syncCardResizeHandleVisibility();
}

function revertCurrentEdit() {
  if (!currentEditorView || currentSavedContent === null) return;
  if (!confirm("放弃当前改动，还原为上次保存的内容？")) return;
  currentEditorView.dispatch({
    changes: { from: 0, to: currentEditorView.state.doc.length, insert: currentSavedContent },
  });
  setDirty(false);
}

async function saveCurrentEdit() {
  if (!currentEditorView || currentPreviewPath === null) return;
  const path = currentPreviewPath;
  const content = currentEditorView.state.doc.toString();
  els.btnPreviewSave.disabled = true;
  try {
    await invoke("save_file_content", { path, content, overridePath: lockedWorkspace });
    currentSavedContent = content;
    setDirty(false);
    // The tree's git-status coloring should reflect a just-saved change
    // immediately, not after up to PANEL_POLL_MS — deliberately
    // refreshTreeAndGitStatus(), not the full refreshPanel(): HEAD hasn't
    // moved (a disk save isn't a commit), so the merge view's own gutter
    // decorations don't need refetching, and re-mounting the editor here
    // would reset cursor/scroll right after the user's own save action.
    refreshTreeAndGitStatus();
  } catch (err) {
    // Left exactly as the user typed it on failure — nothing is discarded
    // on a failed write.
    alert(`保存失败: ${err}`);
  } finally {
    els.btnPreviewSave.disabled = false;
  }
}

// Rebuilds the picker's <option>s from the known-workspaces list plus the
// auto-follow sentinel (whose label carries the live-resolved name, when
// there is one, so auto mode stays informative without a second element).
function renderWorkspaceOptions(knownWorkspaces, autoLabel) {
  const select = els.panelWorkspaceSelect;
  select.replaceChildren();

  const autoOption = document.createElement("option");
  autoOption.value = AUTO_OPTION_VALUE;
  autoOption.textContent = autoLabel ? `自动跟随（${autoLabel}）` : "自动跟随当前会话";
  select.appendChild(autoOption);

  let lockedValueFound = lockedWorkspace === null;
  for (const ws of knownWorkspaces) {
    const option = document.createElement("option");
    option.value = ws.path;
    option.textContent = ws.title;
    option.title = ws.path;
    select.appendChild(option);
    if (ws.path === lockedWorkspace) lockedValueFound = true;
  }
  // The locked path was picked from a list that has since changed (e.g. the
  // workspace was removed/archived in the harness) — keep showing it rather
  // than silently falling back, since the directory on disk hasn't gone
  // anywhere; only the picker's own option list is stale.
  if (!lockedValueFound) {
    const staleOption = document.createElement("option");
    staleOption.value = lockedWorkspace;
    staleOption.textContent = lockedWorkspace;
    select.appendChild(staleOption);
  }

  select.value = lockedWorkspace ?? AUTO_OPTION_VALUE;
}

// Split out from refreshPanel so saveCurrentEdit can refresh the tree's
// git-status coloring right after a save without also re-mounting the
// editor it just saved (see the comment on saveCurrentEdit).
async function refreshTreeAndGitStatus() {
  const knownWorkspacesPromise = invoke("get_known_workspaces").catch(() => []);

  // Skipped entirely once the user has a manual pick locked in — there's
  // nothing left to infer. Otherwise re-resolved every refresh, not just
  // once at startup: the harness's own in-page workspace switcher, entirely
  // inside the iframe with no signal reaching this shell directly, can
  // change independently of anything else this shell observes.
  let autoLabel = null;
  if (lockedWorkspace === null) {
    try {
      autoLabel = await invoke("get_active_workspace");
    } catch {
      /* falls through with autoLabel null; the option keeps its static text */
    }
  }
  renderWorkspaceOptions(await knownWorkspacesPromise, autoLabel);

  // Collapse keys are workspace-relative: a different workspace means a
  // fresh tree, so its collapse state starts empty. Covers both the manual
  // picker and the auto-follow re-resolution above (autoLabel is the path
  // get_active_workspace resolved; a transient failure keeps the old state
  // rather than wiping it).
  applyWorkspaceChange(lockedWorkspace ?? autoLabel);

  try {
    const treeArgs = { overridePath: lockedWorkspace };
    const [tree, gitEntries] = await Promise.all([
      invoke("get_workspace_tree", treeArgs),
      invoke("get_git_status", treeArgs),
    ]);
    const gitMap = new Map(gitEntries.map((e) => [e.path, e.status]));
    els.panelTree.replaceChildren();
    if (tree.length === 0) {
      const empty = document.createElement("p");
      empty.className = "muted panel-empty";
      empty.textContent = "空工作区";
      els.panelTree.appendChild(empty);
    } else {
      for (const entry of tree) {
        renderTreeNode(entry, gitMap, els.panelTree);
      }
    }
  } catch (err) {
    els.panelTree.textContent = `无法加载文件树: ${err}`;
  }
}

async function refreshPanel() {
  await refreshTreeAndGitStatus();
  await refreshCurrentPreview();
}

// Cheap enough (one directory walk + one `git status`) to poll rather than
// stand up a real filesystem watcher for this first slice — see the plan
// note on deferring that complexity. Cleared on nothing; the container page
// itself is never torn down, so this interval simply runs for the app's
// whole lifetime.
const PANEL_POLL_MS = 6000;

// ── init ─────────────────────────────────────────────────────────────────

async function init() {
  applyPanelWidth();
  initResizeHandle();
  applyCardFileHeight();
  initCardsResizeHandle();
  syncCardResizeHandleVisibility();
  initWindowChrome();
  // macOS uses the native title bar buttons and native top menu bar; the
  // custom window controls and the in-window hamburger menu are both
  // hidden there (see styles.css body.platform-decorated) and would only
  // double up with chrome that already exists natively.
  if (!IS_MACOS) {
    initWindowControls();
    initAppMenu();
  }

  try {
    const info = await invoke("get_info");
    const bits = [];
    if (info.dshVersion) bits.push(`dsh ${info.dshVersion}`);
    if (info.nodePath) bits.push(`Node ${info.nodePath}`);
    if (info.dshHome) bits.push(`数据目录 ${info.dshHome}`);
    els.footer.textContent = bits.join(" · ");
  } catch {
    /* footer is cosmetic */
  }

  listen("server-status", (event) => render(event.payload));
  listen("terminal-data", (event) => xterm?.write(base64ToBytes(event.payload)));
  listen("terminal-exit", () => {
    terminalSpawned = false;
    xterm?.writeln("\r\n\x1b[90m[进程已结束]\x1b[0m");
  });
  els.btnRetry.addEventListener("click", () => {
    els.btnRetry.disabled = true;
    invoke("start_server")
      .catch((err) => {
        els.errorMessage.textContent = `启动失败: ${err}`;
      })
      .finally(() => {
        els.btnRetry.disabled = false;
      });
  });
  els.btnRestart.addEventListener("click", () => {
    els.btnRestart.disabled = true;
    invoke("restart_server")
      .catch((err) => {
        els.errorMessage.textContent = `重启失败: ${err}`;
      })
      .finally(() => {
        els.btnRestart.disabled = false;
      });
  });
  els.btnLogs.addEventListener("click", toggleLogs);
  els.btnLogsStarting.addEventListener("click", toggleLogsStarting);
  els.btnOpenBrowser.addEventListener("click", () => invoke("open_in_browser"));
  els.btnToolbarFiles.addEventListener("click", toggleDock);
  els.btnToolbarTerminal.addEventListener("click", toggleTerminal);
  els.btnTerminalRestart.addEventListener("click", async () => {
    await invoke("terminal_close").catch(() => {});
    terminalSpawned = false;
    await ensureTerminal();
  });
  els.btnTerminalClose.addEventListener("click", () => {
    invoke("terminal_close").catch(() => {});
    terminalSpawned = false;
    setTerminalOpen(false);
  });
  // Collapses the Files card's tree/picker body without closing the whole
  // dock — independent from #card-file's own close button, per the "each
  // card scrolls/collapses on its own" design.
  els.btnFilesCollapse.addEventListener("click", () => {
    const collapsed = els.cardFiles.classList.toggle("card-collapsed");
    els.btnFilesCollapse
      .querySelector("use")
      .setAttribute("href", collapsed ? "#icon-chevron-down" : "#icon-chevron-up");
    applyCardFileHeight();
    syncCardResizeHandleVisibility();
  });
  els.btnPanelRefresh.addEventListener("click", refreshPanel);
  els.panelWorkspaceSelect.addEventListener("change", () => {
    const value = els.panelWorkspaceSelect.value;
    if (!confirmDiscardIfNeeded()) {
      // The <select>'s own DOM value already changed on click, ahead of
      // this handler — revert it to match the choice actually still in
      // effect, or the control would show a selection the app never adopted.
      els.panelWorkspaceSelect.value = lockedWorkspace ?? AUTO_OPTION_VALUE;
      return;
    }
    lockedWorkspace = value === AUTO_OPTION_VALUE ? null : value;
    if (lockedWorkspace === null) {
      localStorage.removeItem(LOCKED_WORKSPACE_KEY);
    } else {
      localStorage.setItem(LOCKED_WORKSPACE_KEY, lockedWorkspace);
    }
    // An open preview's path is relative to whichever workspace was active
    // when it was opened — re-resolving it against the new one could silently
    // show an unrelated (or nonexistent) file of the same relative path.
    closePreview();
    refreshPanel();
  });
  els.btnPreviewClose.addEventListener("click", closePreview);
  els.btnPreviewSave.addEventListener("click", saveCurrentEdit);
  els.btnPreviewRevert.addEventListener("click", revertCurrentEdit);

  els.btnUpdateDismiss.addEventListener("click", () => {
    els.updateBanner.classList.add("hidden");
  });
  els.btnUpdateInstall.addEventListener("click", () => {
    els.btnUpdateInstall.disabled = true;
    els.btnUpdateInstall.textContent = "正在更新…";
    els.btnUpdateDismiss.disabled = true;
    // On success this relaunches the app (the window disappears); a caught
    // error means the update didn't apply, so restore the button for retry.
    invoke("install_update").catch((err) => {
      els.btnUpdateInstall.disabled = false;
      els.btnUpdateInstall.textContent = "立即更新";
      els.btnUpdateDismiss.disabled = false;
      els.updateText.textContent = `更新失败: ${err}`;
    });
  });
  checkForUpdate();
  initProviderTip();

  setInterval(refreshPanel, PANEL_POLL_MS);
  await refresh();
  await refreshPanel();
}

async function checkForUpdate() {
  try {
    const update = await invoke("check_for_update");
    if (!update) return;
    els.updateText.textContent = `发现新版本 ${update.version}`;
    els.updateBanner.classList.remove("hidden");
  } catch {
    /* update check is best-effort; silent failure keeps the boot page usable offline */
  }
}

init();
