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
  btnToolbarPlugins: document.getElementById("btn-toolbar-plugins"),
  dockViewFiles: document.getElementById("dock-view-files"),
  cardTerminal: document.getElementById("card-terminal"),
  terminalContainer: document.getElementById("terminal-container"),
  btnTerminalRestart: document.getElementById("btn-terminal-restart"),
  btnTerminalClose: document.getElementById("btn-terminal-close"),
  pluginMarketOverlay: document.getElementById("plugin-market-overlay"),
  btnPluginMarketClose: document.getElementById("btn-plugin-market-close"),
  pluginMarketBrowse: document.getElementById("plugin-market-browse"),
  pluginMarketSearch: document.getElementById("plugin-market-search"),
  pluginMarketCategory: document.getElementById("plugin-market-category"),
  btnPluginMarketSort: document.getElementById("btn-plugin-market-sort"),
  pluginMarketSortIcon: document.getElementById("plugin-market-sort-icon"),
  pluginMarketStatus: document.getElementById("plugin-market-status"),
  pluginMarketGrid: document.getElementById("plugin-market-grid"),
  pluginMarketConfirm: document.getElementById("plugin-market-confirm"),
  btnPluginConfirmBack: document.getElementById("btn-plugin-confirm-back"),
  pluginConfirmName: document.getElementById("plugin-confirm-name"),
  pluginConfirmDesc: document.getElementById("plugin-confirm-desc"),
  pluginConfirmMeta: document.getElementById("plugin-confirm-meta"),
  pluginConfirmCmd: document.getElementById("plugin-confirm-cmd"),
  pluginConfirmPnpmMissing: document.getElementById("plugin-confirm-pnpm-missing"),
  btnPluginInstallPnpm: document.getElementById("btn-plugin-install-pnpm"),
  btnPluginInstallPnpmIcon: document.getElementById("btn-plugin-install-pnpm-icon"),
  btnPluginInstallPnpmLabel: document.getElementById("btn-plugin-install-pnpm-label"),
  btnPluginConfirmSource: document.getElementById("btn-plugin-confirm-source"),
  btnPluginConfirmInstall: document.getElementById("btn-plugin-confirm-install"),
  btnPluginConfirmInstallIcon: document.getElementById("btn-plugin-confirm-install-icon"),
  btnPluginConfirmInstallLabel: document.getElementById("btn-plugin-confirm-install-label"),
  pluginInstallLogWrap: document.getElementById("plugin-install-log-wrap"),
  pluginInstallLog: document.getElementById("plugin-install-log"),
  btnPluginInstallLogClose: document.getElementById("btn-plugin-install-log-close"),
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

// ── i18n ─────────────────────────────────────────────────────────────────
//
// This shell (index.html + this file) is its own bilingual surface,
// deliberately independent of the harness iframe's own locale.preference
// setting: that page is a plain remote page with zero Tauri IPC access (see
// lib.rs's module doc comment), so there's no channel to read its setting
// through, and no reason to invent one. Detected straight from
// navigator.language instead — WebView2/WKWebView both set this from the OS
// UI language — using the same "positively-detected English, else Chinese"
// fallback @deepseek-ai/dsh-client-locale itself uses (see GitHub issue
// #23's own analysis of that package). menu.rs/lib.rs/server.rs/panel.rs
// mirror this exact rule on the native (Rust) side of the shell, via
// sys_locale — the two halves detect independently but land on the same
// answer since both read the same OS setting.
const LANG = (navigator.language || "zh").toLowerCase().startsWith("en") ? "en" : "zh";
document.documentElement.lang = LANG === "en" ? "en" : "zh-CN";

const STRINGS = {
  zh: {
    menu: "菜单",
    openInBrowser: "在浏览器中打开",
    restartService: "重启服务",
    appMenuOpenDataDir: "打开数据目录",
    autostartToggle: "开机自动启动",
    quit: "退出",
    pluginMarket: "插件市场",
    tagline: "探索未至之境",
    terminal: "终端",
    diffComingSoon: "Diff（即将推出）",
    files: "文件",
    minimize: "最小化",
    maximize: "最大化",
    restore: "还原",
    close: "关闭",
    updateNow: "立即更新",
    dismiss: "忽略",
    startingTitle: "正在启动 DeepSeek Harness…",
    preparingLocalService: "准备本地服务",
    viewLogs: "查看日志",
    hideLogs: "隐藏日志",
    providerTip:
      "首次使用提示：默认使用 DeepSeek 官方模型。如需接入 OpenAI / Anthropic / Gemini " +
      "等其他模型，进入后可在「设置 → 模型 → 添加提供方」中配置，随时可改。",
    gotIt: "知道了",
    startupFailed: "启动失败",
    retry: "重试",
    refreshTreeTitle: "刷新文件树与 Git 状态",
    collapse: "收起",
    chooseWorkspaceTitle: "选择要在文件树中查看的工作区",
    unsavedChangesTitle: "有未保存的改动",
    revert: "还原",
    revertTitle: "放弃改动，还原为已保存内容",
    save: "保存",
    saveTitle: "保存改动 (Ctrl+S)",
    closePreviewTitle: "关闭预览",
    restartTerminalTitle: "重启终端",
    closeTerminalTitle: "关闭终端",
    pluginSearchPlaceholder: "搜索插件名称或描述…",
    sortByStarsTitle: "按 star 数排序",
    sortByStarsAsc: "按 star 数从低到高排序",
    sortByStarsDesc: "按 star 数从高到低排序",
    backToList: "返回列表",
    pluginRiskTitle: "安装第三方插件存在风险。",
    pluginRiskBody:
      "插件会以当前用户权限运行第三方代码，可读取本机文件、使用你的登录凭据、访问网络，安装/审批环节不会对其进行沙箱隔离。来源仓库的构建脚本也会在安装时执行。仅安装你信任的来源。",
    commandToRun: "将执行的命令",
    pnpmNotFoundTitle: "未检测到 pnpm。",
    pnpmNotFoundBody: "插件安装依赖 pnpm，请先安装后再继续。",
    installPnpmOneClick: "一键安装 pnpm",
    installing: "安装中…",
    viewSource: "查看来源",
    viewSourceTitle: "在浏览器中查看来源仓库",
    confirmInstall: "确认安装",
    checkingEnvironment: "正在检测环境…",
    installLog: "安装日志",

    unknownError: "未知错误",
    serviceStopped: (code) => `服务已停止（exit ${code}）。`,
    statusFetchFailed: (err) => `无法获取状态: ${err}`,
    logReadFailed: (err) => `无法读取日志: ${err}`,
    allCategories: "全部分类",
    invalidResponseFormat: "响应格式不正确",
    pluginCountStatus: (shown, total, capped) =>
      `${shown} / ${total} 个插件${capped ? `（仅显示前 ${PLUGIN_GRID_RENDER_CAP} 个，请搜索缩小范围）` : ""}`,
    noPluginsMatched: "没有匹配的插件。",
    install: "安装",
    loadingCatalog: "正在加载插件目录…",
    catalogLoadFailed: (err) => `插件目录加载失败: ${err}`,
    installStartFailed: (err) => `启动安装失败: ${err}`,
    sourceMeta: (url, stars) => `来源：${url} · ★ ${stars}`,
    processExited: "[进程已结束]",
    terminalStartFailed: (err) => `终端启动失败: ${err}`,
    installComplete: "安装完成。",
    installFailedExit: (code) => `安装失败（exit ${code}）。`,
    pnpmInstallComplete: "pnpm 安装完成。",
    pnpmInstallFailedExit: (code) => `pnpm 安装失败（exit ${code}）。`,
    startFailed: (err) => `启动失败: ${err}`,
    restartFailed: (err) => `重启失败: ${err}`,
    updating: "正在更新…",
    updateFailed: (err) => `更新失败: ${err}`,
    newVersionFound: (version) => `发现新版本 ${version}`,
    confirmDiscardChanges: "有未保存的改动，确定要放弃吗？",
    cannotReadHistoricalContent: "无法读取此文件的历史内容",
    binaryFileNoPreview: "二进制文件，无法预览",
    fileTooLarge: (mb) => `文件过大（${mb} MB），未加载预览`,
    loading: "加载中…",
    previewLoadFailed: (err) => `预览加载失败: ${err}`,
    confirmRevert: "放弃当前改动，还原为上次保存的内容？",
    saveFailed: (err) => `保存失败: ${err}`,
    autoFollowWithLabel: (label) => `自动跟随（${label}）`,
    autoFollowSession: "自动跟随当前会话",
    emptyWorkspace: "空工作区",
    treeLoadFailed: (err) => `无法加载文件树: ${err}`,
    fileNotInKnownWorkspace: "该文件不属于任何已知工作区",
    dataDirLabel: (path) => `数据目录 ${path}`,
  },
  en: {
    menu: "Menu",
    openInBrowser: "Open in Browser",
    restartService: "Restart Service",
    appMenuOpenDataDir: "Open Data Folder",
    autostartToggle: "Launch at Startup",
    quit: "Quit",
    pluginMarket: "Plugin Market",
    tagline: "Explore the uncharted.",
    terminal: "Terminal",
    diffComingSoon: "Diff (Coming Soon)",
    files: "Files",
    minimize: "Minimize",
    maximize: "Maximize",
    restore: "Restore",
    close: "Close",
    updateNow: "Update Now",
    dismiss: "Dismiss",
    startingTitle: "Starting DeepSeek Harness…",
    preparingLocalService: "Preparing local service",
    viewLogs: "View Logs",
    hideLogs: "Hide Logs",
    providerTip:
      "First time here: DeepSeek's official models are used by default. To add OpenAI / Anthropic / " +
      "Gemini or other providers, go to Settings → Models → Add Provider after launch — you can change this anytime.",
    gotIt: "Got It",
    startupFailed: "Startup Failed",
    retry: "Retry",
    refreshTreeTitle: "Refresh file tree and Git status",
    collapse: "Collapse",
    chooseWorkspaceTitle: "Choose which workspace to show in the file tree",
    unsavedChangesTitle: "Unsaved changes",
    revert: "Revert",
    revertTitle: "Discard changes and revert to the last saved version",
    save: "Save",
    saveTitle: "Save changes (Ctrl+S)",
    closePreviewTitle: "Close Preview",
    restartTerminalTitle: "Restart Terminal",
    closeTerminalTitle: "Close Terminal",
    pluginSearchPlaceholder: "Search plugin name or description…",
    sortByStarsTitle: "Sort by star count",
    sortByStarsAsc: "Sort by star count, low to high",
    sortByStarsDesc: "Sort by star count, high to low",
    backToList: "Back to List",
    pluginRiskTitle: "Installing third-party plugins carries risk.",
    pluginRiskBody:
      "Plugins run third-party code with your current user permissions — they can read local files, use your login " +
      "credentials, and access the network. Installation isn't sandboxed, and the source repo's build scripts run " +
      "during install too. Only install sources you trust.",
    commandToRun: "Command to run",
    pnpmNotFoundTitle: "pnpm not found.",
    pnpmNotFoundBody: "Plugin installation requires pnpm — install it first to continue.",
    installPnpmOneClick: "Install pnpm",
    installing: "Installing…",
    viewSource: "View Source",
    viewSourceTitle: "View the source repository in your browser",
    confirmInstall: "Confirm Install",
    checkingEnvironment: "Checking environment…",
    installLog: "Install Log",

    unknownError: "Unknown error",
    serviceStopped: (code) => `Service stopped (exit ${code}).`,
    statusFetchFailed: (err) => `Failed to fetch status: ${err}`,
    logReadFailed: (err) => `Failed to read log: ${err}`,
    allCategories: "All Categories",
    invalidResponseFormat: "Invalid response format",
    pluginCountStatus: (shown, total, capped) =>
      `${shown} / ${total} plugins${capped ? ` (showing first ${PLUGIN_GRID_RENDER_CAP} — search to narrow down)` : ""}`,
    noPluginsMatched: "No matching plugins.",
    install: "Install",
    loadingCatalog: "Loading plugin catalog…",
    catalogLoadFailed: (err) => `Failed to load plugin catalog: ${err}`,
    installStartFailed: (err) => `Failed to start install: ${err}`,
    sourceMeta: (url, stars) => `Source: ${url} · ★ ${stars}`,
    processExited: "[process exited]",
    terminalStartFailed: (err) => `Failed to start terminal: ${err}`,
    installComplete: "Install complete.",
    installFailedExit: (code) => `Install failed (exit ${code}).`,
    pnpmInstallComplete: "pnpm installed.",
    pnpmInstallFailedExit: (code) => `pnpm install failed (exit ${code}).`,
    startFailed: (err) => `Failed to start: ${err}`,
    restartFailed: (err) => `Failed to restart: ${err}`,
    updating: "Updating…",
    updateFailed: (err) => `Update failed: ${err}`,
    newVersionFound: (version) => `New version ${version} available`,
    confirmDiscardChanges: "You have unsaved changes. Discard them?",
    cannotReadHistoricalContent: "Unable to read this file's historical content",
    binaryFileNoPreview: "Binary file, no preview available",
    fileTooLarge: (mb) => `File too large (${mb} MB) — preview not loaded`,
    loading: "Loading…",
    previewLoadFailed: (err) => `Failed to load preview: ${err}`,
    confirmRevert: "Discard current changes and revert to the last saved version?",
    saveFailed: (err) => `Save failed: ${err}`,
    autoFollowWithLabel: (label) => `Auto-follow (${label})`,
    autoFollowSession: "Auto-follow current session",
    emptyWorkspace: "Empty workspace",
    treeLoadFailed: (err) => `Failed to load file tree: ${err}`,
    fileNotInKnownWorkspace: "This file doesn't belong to any known workspace",
    dataDirLabel: (path) => `Data folder ${path}`,
  },
};

function t(key, ...args) {
  const entry = STRINGS[LANG][key];
  return typeof entry === "function" ? entry(...args) : entry;
}

// Applies every data-i18n[-title|-placeholder] attribute in index.html — the
// static markup ships with Chinese text as its own fallback (never blank),
// so this only needs to run once, after the DOM exists, to swap in whichever
// language LANG resolved to.
function applyStaticTranslations() {
  for (const el of document.querySelectorAll("[data-i18n]")) {
    el.textContent = t(el.getAttribute("data-i18n"));
  }
  for (const el of document.querySelectorAll("[data-i18n-title]")) {
    el.title = t(el.getAttribute("data-i18n-title"));
  }
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) {
    el.placeholder = t(el.getAttribute("data-i18n-placeholder"));
  }
}

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
    box.textContent = t("logReadFailed", err);
  }
}

function toggleLogs() {
  logsVisible = !logsVisible;
  els.logBox.classList.toggle("hidden", !logsVisible);
  els.btnLogs.textContent = logsVisible ? t("hideLogs") : t("viewLogs");
  if (logsVisible) loadLogsInto(els.logBox);
}

function toggleLogsStarting() {
  logsStartingVisible = !logsStartingVisible;
  els.logBoxStarting.classList.toggle("hidden", !logsStartingVisible);
  els.btnLogsStarting.textContent = logsStartingVisible ? t("hideLogs") : t("viewLogs");
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
      els.startingDetail.textContent = status.detail || t("preparingLocalService");
      break;
    case "stopped":
      show("error");
      els.harnessFrame.src = "about:blank";
      els.errorMessage.textContent = t("serviceStopped", status.code ?? "?") + (status.message ? `\n${status.message}` : "");
      break;
    case "error":
      show("error");
      els.harnessFrame.src = "about:blank";
      els.errorMessage.textContent = status.message || t("unknownError");
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
    els.errorMessage.textContent = t("statusFetchFailed", err);
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
  els.btnWinMaximize.title = maximized ? t("restore") : t("maximize");
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

// ── dock view switching ─────────────────────────────────────────────────
//
// One dock (#panel), two mutually exclusive views — Files and Terminal
// share the same toolbar button group for a reason: opening one is meant
// to replace the other, not stack beside it (unlike Files vs. its own
// File-preview card, which *are* meant to sit together — see
// #dock-view-files). Neither is persisted across launches; each toolbar
// button is a deliberate opt-in per session. Only the dock's width (once
// opened) is remembered, via panelWidth/PANEL_WIDTH_KEY above.

// `refresh: false` lets a caller that's about to drive its own, carefully
// ordered refresh (see handleFileMention) switch to the Files view without
// also kicking off this function's own un-awaited refreshPanel() — two
// concurrent, uncoordinated tree renders racing against each other, one of
// which could win before the caller's own state (e.g. currentPreviewPath)
// is actually set, would put the exact same stale-selection race right back
// after fixing it in the caller. Only meaningful for view: "files" —
// opening the terminal view already has its own, independent readiness
// gate (ensureTerminal()) that this option doesn't touch either way.
function setDockView(view, { refresh = true } = {}) {
  // view: "files" | "terminal" | null (closed) — the plugin market lives
  // outside this dock entirely now (see #plugin-market-overlay in
  // index.html and togglePluginMarket below), so it's not a case here.
  const filesOpen = view === "files";
  const terminalOpen = view === "terminal";

  els.btnToolbarFiles.classList.toggle("active", filesOpen);
  els.dockViewFiles.classList.toggle("hidden", !filesOpen);

  els.btnToolbarTerminal.classList.toggle("active", terminalOpen);
  els.cardTerminal.classList.toggle("hidden", !terminalOpen);

  els.panel.classList.toggle("hidden", view === null);
  els.resizePanelContent.classList.toggle("hidden", view === null);

  if (filesOpen && refresh) refreshPanel();
  if (terminalOpen) {
    ensureTerminal().then(() => requestAnimationFrame(fitTerminal));
  }
}

function toggleDock() {
  setDockView(els.btnToolbarFiles.classList.contains("active") ? null : "files");
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
    xterm.writeln(`\r\n\x1b[31m${t("terminalStartFailed", err)}\x1b[0m`);
  }
}

function toggleTerminal() {
  setDockView(els.btnToolbarTerminal.classList.contains("active") ? null : "terminal");
}

// ── plugin market ────────────────────────────────────────────────────────
//
// The catalog is fetched live from awesome-dsh-plugin.com/plugins.json — a
// crowd-submitted "awesome list" (GitHub-PR intake), not a registry this
// project vets. Every entry is exactly as trustworthy as its own GitHub
// repo, no more. Installs still go through the same `dsh plugin --profile
// web add <package>` (server.rs's install_plugin — a thin forward to
// `pnpm` inside the harness's own web-profile directory) as before this
// catalog existed; only the source of *what's offered* changed, not how an
// install actually runs. Because nothing here is pre-vetted, every install
// is gated behind #plugin-market-confirm showing the literal command and a
// standing risk notice — see openConfirmView / installConfirmedPlugin
// below — there is no direct install button on a grid card.
const PLUGIN_CATALOG_URL = "https://awesome-dsh-plugin.com/plugins.json";

// Populated by loadPluginCatalog(); { categories: {id: {en,zh}}, plugins: [...] }.
let pluginCatalog = null;
let pluginCatalogLoadPromise = null;
let pluginMarketSearchDebounce = null;
// The plugin currently shown in the confirm view — installConfirmedPlugin
// reads the command from here rather than from a DOM data-attribute, so
// there's exactly one place ("this session's catalog fetch") a command
// string can come from.
let pluginConfirmTarget = null;

// "desc" doubles as "no explicit sort" — the catalog's own fetched order
// is already highest-star-first (awesome-dsh-plugin.com lists it that
// way), so toggling to "desc" and leaving it untouched read identically.
// Only two states to cycle between (see btnPluginMarketSort's click
// handler) rather than a third "unsorted" one that would just duplicate
// "desc" under a different name.
let pluginSortDirection = "desc";

async function loadPluginCatalog() {
  if (pluginCatalog) return pluginCatalog;
  if (pluginCatalogLoadPromise) return pluginCatalogLoadPromise;
  pluginCatalogLoadPromise = (async () => {
    const res = await fetch(PLUGIN_CATALOG_URL);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    if (!Array.isArray(data.plugins)) throw new Error(t("invalidResponseFormat"));
    pluginCatalog = data;
    return data;
  })();
  try {
    return await pluginCatalogLoadPromise;
  } finally {
    pluginCatalogLoadPromise = null;
  }
}

function pluginCatalogLang() {
  return LANG;
}

function populateCategoryFilter(categories) {
  const lang = pluginCatalogLang();
  els.pluginMarketCategory.replaceChildren();
  const allOpt = document.createElement("option");
  allOpt.value = "";
  allOpt.textContent = t("allCategories");
  els.pluginMarketCategory.appendChild(allOpt);
  for (const [id, names] of Object.entries(categories)) {
    const opt = document.createElement("option");
    opt.value = id;
    opt.textContent = names[lang] || names.en || id;
    els.pluginMarketCategory.appendChild(opt);
  }
}

// Sorts before renderPluginGrid's own PLUGIN_GRID_RENDER_CAP slice runs,
// not after — sorting a result set and then truncating it is the only
// order that makes "ascending" mean anything; truncating first and
// sorting what's left would show the lowest-star entries *of an arbitrary
// first-200 slice*, not the catalog's actual lowest-star entries.
function filteredPlugins() {
  if (!pluginCatalog) return [];
  const query = els.pluginMarketSearch.value.trim().toLowerCase();
  const category = els.pluginMarketCategory.value;
  const lang = pluginCatalogLang();
  const results = pluginCatalog.plugins.filter((p) => {
    if (category && p.category !== category) return false;
    if (!query) return true;
    const desc = (p.description?.[lang] || p.description?.en || "").toLowerCase();
    return p.name.toLowerCase().includes(query) || p.owner.toLowerCase().includes(query) || desc.includes(query);
  });
  // .filter() above already returns a fresh array, so sorting in place
  // here doesn't mutate pluginCatalog.plugins itself.
  results.sort((a, b) => {
    const diff = (a.stars ?? 0) - (b.stars ?? 0);
    return pluginSortDirection === "asc" ? diff : -diff;
  });
  return results;
}

function togglePluginSort() {
  pluginSortDirection = pluginSortDirection === "desc" ? "asc" : "desc";
  const icon = pluginSortDirection === "asc" ? "arrow-up" : "arrow-down";
  els.pluginMarketSortIcon.querySelector("use").setAttribute("href", `#icon-${icon}`);
  els.btnPluginMarketSort.title = pluginSortDirection === "asc" ? t("sortByStarsAsc") : t("sortByStarsDesc");
  renderPluginGrid();
}

// Caps rendered cards per render — 839 entries is fine to filter+sort in
// one pass, but appending 839 DOM nodes at once (or on every keystroke,
// given the search debounce below) is the kind of thing that turns typing
// into visible jank for no benefit: nobody scans past the first couple
// hundred of a result set anyway, sorted or not — this slices whatever
// filteredPlugins() already sorted, never the other way around.
const PLUGIN_GRID_RENDER_CAP = 200;

// GitHub-style star-count abbreviation (20011 → "20k") — the catalog's raw
// integers read fine at card scale for anything under 1000, but a 5-digit
// number next to a single-glyph star icon (see renderPluginGrid) crowds the
// name/owner it shares a row with. One significant digit past the decimal,
// trimmed of a trailing ".0", matches how GitHub itself abbreviates.
function formatStars(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}m`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  return String(n);
}

function renderPluginGrid() {
  const lang = pluginCatalogLang();
  const results = filteredPlugins();
  els.pluginMarketStatus.textContent = t(
    "pluginCountStatus",
    results.length,
    pluginCatalog.plugins.length,
    results.length > PLUGIN_GRID_RENDER_CAP
  );
  if (results.length === 0) {
    renderPluginGridPlaceholder("inbox", t("noPluginsMatched"), false);
    return;
  }
  els.pluginMarketGrid.replaceChildren();
  const categoryNames = pluginCatalog.categories;
  for (const plugin of results.slice(0, PLUGIN_GRID_RENDER_CAP)) {
    const card = document.createElement("div");
    card.className = "plugin-card";

    const top = document.createElement("div");
    top.className = "plugin-card-top";
    const name = document.createElement("span");
    name.className = "plugin-card-name";
    name.title = `${plugin.owner}/${plugin.name}`;
    const owner = document.createElement("span");
    owner.className = "plugin-card-owner";
    owner.textContent = `${plugin.owner}/`;
    name.appendChild(owner);
    name.appendChild(document.createTextNode(plugin.name));
    const stars = document.createElement("span");
    stars.className = "plugin-card-stars";
    stars.title = `${plugin.stars ?? 0} stars`;
    stars.appendChild(iconEl("star", "plugin-card-stars-icon"));
    stars.appendChild(document.createTextNode(formatStars(plugin.stars ?? 0)));
    top.appendChild(name);
    top.appendChild(stars);

    const desc = document.createElement("p");
    desc.className = "plugin-card-desc";
    const descText = plugin.description?.[lang] || plugin.description?.en || "";
    desc.textContent = descText;
    desc.title = descText;

    const foot = document.createElement("div");
    foot.className = "plugin-card-foot";
    const catNames = categoryNames[plugin.category];
    const cat = document.createElement("span");
    cat.className = "plugin-card-category";
    cat.textContent = (catNames && (catNames[lang] || catNames.en)) || plugin.category;
    const btn = document.createElement("button");
    btn.className = "plugin-card-install-btn";
    btn.appendChild(iconEl("download", "icon"));
    btn.appendChild(document.createTextNode(t("install")));
    btn.addEventListener("click", () => openConfirmView(plugin));
    foot.appendChild(cat);
    foot.appendChild(btn);

    card.appendChild(top);
    card.appendChild(desc);
    card.appendChild(foot);
    els.pluginMarketGrid.appendChild(card);
  }
}

function onPluginMarketFilterChange() {
  clearTimeout(pluginMarketSearchDebounce);
  pluginMarketSearchDebounce = setTimeout(renderPluginGrid, 120);
}

// The catalog's own `install` field is a full CLI invocation string (`dsh
// plugin --profile web add <package>`) meant for a human to copy-paste —
// install_plugin (server.rs) instead takes just `<package>` as a single
// arg it appends itself, so the trailing token is all that's forwarded.
function packageFromInstallCommand(installCmd) {
  const parts = installCmd.trim().split(/\s+/);
  return parts[parts.length - 1];
}

function openConfirmView(plugin) {
  pluginConfirmTarget = plugin;
  const lang = pluginCatalogLang();
  els.pluginConfirmName.textContent = `${plugin.owner}/${plugin.name}`;
  els.pluginConfirmDesc.textContent = plugin.description?.[lang] || plugin.description?.en || "";
  els.pluginConfirmMeta.textContent = t("sourceMeta", plugin.url, formatStars(plugin.stars ?? 0));
  els.pluginConfirmCmd.textContent = plugin.install;
  els.pluginMarketBrowse.classList.add("hidden");
  els.pluginMarketConfirm.classList.remove("hidden");
  refreshPnpmGate();
}

// The install button carries an icon (see #btn-plugin-confirm-install-icon
// in index.html) alongside its label, so its busy/idle states can't just
// overwrite textContent — that would wipe the icon along with the old
// text. "loader" swaps in a spinning placeholder for both the checking and
// installing states; "download" is the one idle icon, since idle always
// means "确认安装" regardless of which busy state preceded it.
function setInstallButtonState(icon, label) {
  els.btnPluginConfirmInstallIcon.querySelector("use").setAttribute("href", `#icon-${icon}`);
  els.btnPluginConfirmInstallIcon.classList.toggle("icon-spin", icon === "loader");
  els.btnPluginConfirmInstallLabel.textContent = label;
}

// `dsh plugin add` shells out to pnpm (see install_plugin's own doc
// comment in server.rs) — without it, "确认安装" would just fail on a raw
// OS "pnpm not found" error. Gates the install button on a live probe
// (check_pnpm_available) instead, showing #plugin-confirm-pnpm-missing
// with a one-click fix when it's absent. Re-run after installPnpm
// finishes, not just once on open, so the button unlocks the moment pnpm
// actually becomes available without the user having to reopen the dialog.
async function refreshPnpmGate() {
  els.btnPluginConfirmInstall.disabled = true;
  setInstallButtonState("loader", t("checkingEnvironment"));
  els.pluginConfirmPnpmMissing.classList.add("hidden");
  let available = false;
  try {
    available = await invoke("check_pnpm_available");
  } catch {
    // Treat a failed probe itself as "unknown" — the confirm button stays
    // gated (safer default) and the plugin's own install attempt will
    // surface whatever the real problem is either way.
  }
  // The user may have already navigated away from this exact plugin (or
  // closed the dialog) by the time the async probe above resolves — only
  // touch the confirm view's own controls if we're still looking at it.
  if (els.pluginMarketConfirm.classList.contains("hidden")) return;
  els.pluginConfirmPnpmMissing.classList.toggle("hidden", available);
  els.btnPluginConfirmInstall.disabled = !available;
  setInstallButtonState("download", t("confirmInstall"));
}

function closeConfirmView() {
  pluginConfirmTarget = null;
  els.pluginMarketConfirm.classList.add("hidden");
  els.pluginMarketBrowse.classList.remove("hidden");
}

// Same centered icon+label shape as .plugin-market-empty (renderPluginGrid)
// — loading and "nothing here" read as the same kind of placeholder, just
// with a spinning loader instead of a static inbox glyph and no fixed grid
// membership of its own to worry about clearing later (openPluginMarket
// always replaces this via renderPluginGrid or its own catch block before
// the user can interact with anything else).
function renderPluginGridPlaceholder(icon, label, spin) {
  const wrap = document.createElement("div");
  wrap.className = "panel-empty plugin-market-empty";
  const iconNode = iconEl(icon, "plugin-market-empty-icon");
  if (spin) iconNode.classList.add("icon-spin");
  wrap.appendChild(iconNode);
  const p = document.createElement("p");
  p.className = "muted";
  p.textContent = label;
  wrap.appendChild(p);
  els.pluginMarketGrid.replaceChildren(wrap);
}

async function openPluginMarket() {
  els.pluginMarketOverlay.classList.remove("hidden");
  els.btnToolbarPlugins.classList.add("active");
  closeConfirmView();
  if (!pluginCatalog) {
    els.pluginMarketStatus.textContent = "";
    renderPluginGridPlaceholder("loader", t("loadingCatalog"), true);
  }
  try {
    const data = await loadPluginCatalog();
    populateCategoryFilter(data.categories);
    renderPluginGrid();
  } catch (err) {
    els.pluginMarketStatus.textContent = "";
    renderPluginGridPlaceholder("alert-circle", t("catalogLoadFailed", err), false);
  }
}

function closePluginMarket() {
  els.pluginMarketOverlay.classList.add("hidden");
  els.btnToolbarPlugins.classList.remove("active");
}

function togglePluginMarket() {
  if (els.pluginMarketOverlay.classList.contains("hidden")) {
    openPluginMarket();
  } else {
    closePluginMarket();
  }
}

function showPluginInstallLog() {
  els.pluginInstallLog.textContent = "";
  els.pluginInstallLogWrap.classList.remove("hidden");
}

function appendPluginInstallLog(text) {
  els.pluginInstallLog.textContent += (els.pluginInstallLog.textContent ? "\n" : "") + text;
  els.pluginInstallLog.scrollTop = els.pluginInstallLog.scrollHeight;
}

// The "plugin-install" event listener (see its listen() call below) only
// ever hears about the one dsh child process this shell can run at a time
// (install_plugin's own single-flight guard in server.rs) — it resets
// whichever button this was pointed at once a Done event arrives, without
// Done itself carrying the package name back.
let pluginInstallButton = null;

function resetPluginInstallButton() {
  if (!pluginInstallButton) return;
  pluginInstallButton.disabled = false;
  setInstallButtonState("download", t("confirmInstall"));
  pluginInstallButton = null;
}

// Fires only from the confirm view's own button — never directly off a
// grid card — so every install has already shown its exact command next to
// the standing third-party-code warning before this runs.
async function installConfirmedPlugin() {
  const plugin = pluginConfirmTarget;
  if (!plugin) return;
  const pkg = packageFromInstallCommand(plugin.install);
  els.btnPluginConfirmInstall.disabled = true;
  setInstallButtonState("loader", t("installing"));
  pluginInstallButton = els.btnPluginConfirmInstall;
  showPluginInstallLog();
  appendPluginInstallLog(`$ dsh plugin --profile web add ${pkg}`);
  try {
    await invoke("install_plugin", { package: pkg });
  } catch (err) {
    appendPluginInstallLog(t("installStartFailed", err));
    resetPluginInstallButton();
  }
  // On success, re-enabling the button happens in the "plugin-install"
  // event listener below, keyed on the Done event — the invoke() call
  // above only confirms the child process *started*, not that `pnpm add`
  // itself finished.
}

// The "pnpm-install" event listener (see its listen() call below) only
// ever hears about the one `npm install -g pnpm` child this shell can run
// at a time (install_pnpm's own single-flight guard in server.rs) — same
// button-tracking shape as pluginInstallButton above, kept as its own
// variable since installing pnpm and installing a plugin are independent
// actions that can't collide but also shouldn't share state.
let pnpmInstallButton = null;

// Same icon+label split as setInstallButtonState above, for the smaller
// "一键安装 pnpm" button — its icon only ever toggles between its one idle
// state (download) and busy (spinning loader), no third state to name.
function setPnpmButtonBusy(busy) {
  els.btnPluginInstallPnpmIcon
    .querySelector("use")
    .setAttribute("href", busy ? "#icon-loader" : "#icon-download");
  els.btnPluginInstallPnpmIcon.classList.toggle("icon-spin", busy);
  els.btnPluginInstallPnpmLabel.textContent = busy ? t("installing") : t("installPnpmOneClick");
}

function resetPnpmInstallButton() {
  if (!pnpmInstallButton) return;
  pnpmInstallButton.disabled = false;
  setPnpmButtonBusy(false);
  pnpmInstallButton = null;
}

// Fires only from #plugin-confirm-pnpm-missing's own button. Re-probes via
// refreshPnpmGate() once the child exits successfully, so "确认安装"
// unlocks immediately rather than requiring the user to close and reopen
// the dialog to notice pnpm is now there.
async function installPnpm() {
  els.btnPluginInstallPnpm.disabled = true;
  setPnpmButtonBusy(true);
  pnpmInstallButton = els.btnPluginInstallPnpm;
  showPluginInstallLog();
  appendPluginInstallLog("$ npm install -g pnpm");
  try {
    await invoke("install_pnpm");
  } catch (err) {
    appendPluginInstallLog(t("installStartFailed", err));
    resetPnpmInstallButton();
  }
  // On success, re-enabling the button and re-probing pnpm both happen in
  // the "pnpm-install" event listener below, keyed on the Done event.
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
  return confirm(t("confirmDiscardChanges"));
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
      els.panelPreviewBody.textContent = t("cannotReadHistoricalContent");
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
    els.panelPreviewBody.textContent = t("binaryFileNoPreview");
    return;
  }
  if (current.kind === "tooLarge") {
    els.panelPreviewBody.textContent = t("fileTooLarge", (current.bytes / 1024 / 1024).toFixed(1));
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
  loading.textContent = t("loading");
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
    els.panelPreviewBody.textContent = t("previewLoadFailed", err);
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
  if (!confirm(t("confirmRevert"))) return;
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
    alert(t("saveFailed", err));
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
  autoOption.textContent = autoLabel ? t("autoFollowWithLabel", autoLabel) : t("autoFollowSession");
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
      empty.textContent = t("emptyWorkspace");
      els.panelTree.appendChild(empty);
    } else {
      for (const entry of tree) {
        renderTreeNode(entry, gitMap, els.panelTree);
      }
    }
  } catch (err) {
    els.panelTree.textContent = t("treeLoadFailed", err);
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

// ── file-mention bridge (harness iframe → dock) ─────────────────────────
//
// lib.rs's inject_file_mention_bridge injects a capture-phase click
// listener directly into the harness iframe's document (WebView2's
// AddScriptToExecuteOnDocumentCreated) — the harness itself has no
// postMessage channel of its own and isn't this repo's source to add one
// to. That injected script intercepts a file-mention button's default
// "open" action and posts {source:"dsh-desktop", type:"open-file-mention",
// path: "<absolute path>"} to window.top instead. This listener is the
// other end of that bridge.
//
// The path arrives absolute (it's the button's own title attribute, an OS
// path), but every panel command (get_workspace_tree/get_editable_preview/…)
// takes a path relative to whichever workspace root is active — see
// get_editable_preview's doc comment in lib.rs. So the workspace the file
// belongs to has to be resolved here, from the absolute path, before
// showPreview can be called at all.

function normalizeForCompare(p) {
  return p.replace(/\\/g, "/").toLowerCase();
}

// Longest-root-prefix wins, so a workspace nested inside another workspace's
// directory (however unusual) still resolves to the more specific one
// rather than whichever happened to come first in the list.
function resolveWorkspaceForPath(knownWorkspaces, absPath) {
  const target = normalizeForCompare(absPath);
  let best = null;
  for (const ws of knownWorkspaces) {
    const root = normalizeForCompare(ws.path).replace(/\/+$/, "");
    if (target === root || target.startsWith(root + "/")) {
      if (!best || root.length > normalizeForCompare(best.path).length) best = ws;
    }
  }
  return best;
}

// Same persistence side effects as the manual workspace picker's own change
// handler above — just driven programmatically instead of by a user
// selecting an <option>. Deliberately skips that handler's own
// confirmDiscardIfNeeded() gate: showPreview() (called right after by
// handleFileMention) already runs that same check against the file this
// click is actually trying to open, so gating here too would just ask twice
// for one discard decision.
function lockWorkspace(path) {
  lockedWorkspace = path;
  localStorage.setItem(LOCKED_WORKSPACE_KEY, path);
  els.panelWorkspaceSelect.value = path;
}

// Reveals a just-selected tree row: expands every collapsed ancestor
// (renderTreeNode's own hasChildren click handler is the only thing that
// ever collapses one — a user fold from an earlier look at the tree) so the
// row is actually in the visible/scrollable flow, not sitting inside a
// display:none .tree-children, then scrolls it into view.
function revealSelectedTreeRow() {
  const row = els.panelTree.querySelector(".tree-row-selected");
  if (!row) return;
  let node = row.parentElement;
  while (node && node !== els.panelTree) {
    if (node.classList.contains("tree-children")) {
      node.classList.remove("collapsed");
      const caretRow = node.previousElementSibling;
      if (caretRow) caretRow.classList.add("tree-expanded");
    }
    node = node.parentElement;
  }
  row.scrollIntoView({ block: "nearest" });
}

async function handleFileMention(absPath) {
  setDockView("files", { refresh: false });

  const knownWorkspaces = await invoke("get_known_workspaces").catch(() => []);
  const workspace = resolveWorkspaceForPath(knownWorkspaces, absPath);
  if (!workspace) {
    // No known workspace claims this path — nothing to lock onto or
    // convert a relative path against; surface it the same way an
    // unreadable preview already does rather than silently doing nothing.
    // Tears down whatever was previously open the same way closePreview()
    // does — otherwise a stale editor instance (and its currentPreviewPath)
    // would linger and get silently re-fetched/re-mounted by the next poll,
    // clobbering this error message with unrelated old content.
    currentPreviewPath = null;
    destroyEditor();
    els.panelPreviewTitle.textContent = absPath;
    els.panelPreviewTitle.title = absPath;
    els.cardFile.classList.remove("hidden");
    els.panelPreviewBody.textContent = t("fileNotInKnownWorkspace");
    return;
  }

  const root = workspace.path.replace(/\\/g, "/").replace(/\/+$/, "");
  const relPath = absPath.replace(/\\/g, "/").slice(root.length).replace(/^\/+/, "");

  if (workspace.path !== lockedWorkspace) {
    lockWorkspace(workspace.path);
  }
  // showPreview() must land first — it's the only place that sets
  // currentPreviewPath, and renderTreeNode (inside refreshTreeAndGitStatus)
  // marks .tree-row-selected by comparing against that same value. Doing
  // this in the other order renders the tree against whatever was
  // *previously* open, so revealSelectedTreeRow() below would find a stale
  // row (or none at all) instead of the file this click just opened.
  await showPreview(relPath);
  await refreshTreeAndGitStatus();
  revealSelectedTreeRow();
}

window.addEventListener("message", (event) => {
  // Only the harness iframe itself is a legitimate sender — the injected
  // script runs inside that document and nowhere else, and nothing else in
  // this page's world has a reason to post this message shape.
  if (event.source !== els.harnessFrame.contentWindow) return;
  const data = event.data;
  if (!data || data.source !== "dsh-desktop" || data.type !== "open-file-mention") return;
  if (typeof data.path !== "string" || !data.path) return;
  handleFileMention(data.path);
});

// ── init ─────────────────────────────────────────────────────────────────

async function init() {
  applyStaticTranslations();
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
    if (info.dshHome) bits.push(t("dataDirLabel", info.dshHome));
    els.footer.textContent = bits.join(" · ");
  } catch {
    /* footer is cosmetic */
  }

  listen("server-status", (event) => render(event.payload));
  listen("terminal-data", (event) => xterm?.write(base64ToBytes(event.payload)));
  listen("terminal-exit", () => {
    terminalSpawned = false;
    xterm?.writeln(`\r\n\x1b[90m${t("processExited")}\x1b[0m`);
  });
  listen("plugin-install", (event) => {
    const payload = event.payload;
    if (payload.state === "line") {
      appendPluginInstallLog(payload.text);
    } else if (payload.state === "done") {
      appendPluginInstallLog(payload.success ? t("installComplete") : t("installFailedExit", payload.code ?? "?"));
      resetPluginInstallButton();
    }
  });
  listen("pnpm-install", (event) => {
    const payload = event.payload;
    if (payload.state === "line") {
      appendPluginInstallLog(payload.text);
    } else if (payload.state === "done") {
      appendPluginInstallLog(payload.success ? t("pnpmInstallComplete") : t("pnpmInstallFailedExit", payload.code ?? "?"));
      resetPnpmInstallButton();
      // Re-probe regardless of success/failure — cheap, and covers the case
      // where npm reported non-zero but pnpm actually ended up on PATH
      // anyway (e.g. a post-install warning unrelated to the binary itself).
      if (!els.pluginMarketConfirm.classList.contains("hidden")) refreshPnpmGate();
    }
  });
  els.btnRetry.addEventListener("click", () => {
    els.btnRetry.disabled = true;
    invoke("start_server")
      .catch((err) => {
        els.errorMessage.textContent = t("startFailed", err);
      })
      .finally(() => {
        els.btnRetry.disabled = false;
      });
  });
  els.btnRestart.addEventListener("click", () => {
    els.btnRestart.disabled = true;
    invoke("restart_server")
      .catch((err) => {
        els.errorMessage.textContent = t("restartFailed", err);
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
  els.btnToolbarPlugins.addEventListener("click", togglePluginMarket);
  els.btnPluginMarketClose.addEventListener("click", closePluginMarket);
  els.pluginMarketOverlay.addEventListener("click", (e) => {
    if (e.target === els.pluginMarketOverlay) closePluginMarket();
  });
  els.pluginMarketSearch.addEventListener("input", onPluginMarketFilterChange);
  els.pluginMarketCategory.addEventListener("change", renderPluginGrid);
  els.btnPluginMarketSort.addEventListener("click", togglePluginSort);
  els.btnPluginConfirmBack.addEventListener("click", closeConfirmView);
  els.btnPluginConfirmSource.addEventListener("click", () => {
    if (pluginConfirmTarget) invoke("open_external_url", { url: pluginConfirmTarget.url }).catch(() => {});
  });
  els.btnPluginConfirmInstall.addEventListener("click", installConfirmedPlugin);
  els.btnPluginInstallPnpm.addEventListener("click", installPnpm);
  els.btnPluginInstallLogClose.addEventListener("click", () => {
    els.pluginInstallLogWrap.classList.add("hidden");
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !els.pluginMarketOverlay.classList.contains("hidden")) {
      closePluginMarket();
    }
  });
  els.btnTerminalRestart.addEventListener("click", async () => {
    await invoke("terminal_close").catch(() => {});
    terminalSpawned = false;
    await ensureTerminal();
  });
  els.btnTerminalClose.addEventListener("click", () => {
    invoke("terminal_close").catch(() => {});
    terminalSpawned = false;
    setDockView(null);
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
    els.btnUpdateInstall.textContent = t("updating");
    els.btnUpdateDismiss.disabled = true;
    // On success this relaunches the app (the window disappears); a caught
    // error means the update didn't apply, so restore the button for retry.
    invoke("install_update").catch((err) => {
      els.btnUpdateInstall.disabled = false;
      els.btnUpdateInstall.textContent = t("updateNow");
      els.btnUpdateDismiss.disabled = false;
      els.updateText.textContent = t("updateFailed", err);
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
    els.updateText.textContent = t("newVersionFound", update.version);
    els.updateBanner.classList.remove("hidden");
  } catch {
    /* update check is best-effort; silent failure keeps the boot page usable offline */
  }
}

init();
