//! dsh server manager.
//!
//! Responsibilities:
//! - resolve the Node executable (explicit override → bundled runtime → PATH)
//! - resolve the `dsh` `lib/bin.js` (explicit override → bundled runtime →
//!   per-user managed runtime, installed on first use)
//! - probe `127.0.0.1:3080` and attach to an already-running harness instead
//!   of spawning a second instance (avoids concurrent writers on `~/.dsh`)
//! - spawn `node <bin> web --port …` and discover the real URL from the
//!   printed `dsh web: http://127.0.0.1:<port>` line
//! - navigate the main webview to the URL, watch the process, auto-restart
//!   once per 60s window on unexpected exit, and surface errors to the boot page
//! - clean up the whole process tree (`taskkill /T /F`) on stop

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Tray icon id, shared with `menu::build_tray`'s `TrayIconBuilder::with_id`
/// so `set_status` can look the tray up by id to update its tooltip.
pub const TRAY_ID: &str = "main-tray";

pub const DEFAULT_PORT: u16 = 3080;

/// Default npm version spec for the managed `@deepseek-ai/dsh` runtime.
const DSH_VERSION_DEFAULT: &str = "0.1.0-rc.6";
/// Marker found verbatim in the harness index page (served uncompressed).
const INDEX_MARKER: &str = "DeepSeek Harness";
/// Max lines kept in the in-memory log ring buffer.
const LOG_CAP: usize = 400;
/// The URL line printed by the web profile (`dsh-web-app`, `printUrl: true`).
const URL_PREFIX: &str = "dsh web: http://127.0.0.1:";
/// Minimum gap between automatic restarts of a crashing server.
const AUTO_RESTART_MIN_GAP: Duration = Duration::from_secs(60);
/// How long to wait for the process to print its ready URL line before
/// surfacing a stuck-start error. Generous: first-run installs and cold
/// starts can be slow, but an upstream format change (see `URL_PREFIX`)
/// would otherwise hang here forever with no feedback on the boot page.
const READY_TIMEOUT: Duration = Duration::from_secs(45);
/// Substring of the error `@deepseek-ai/dsh-app-boot`'s `ensureSymlink`
/// throws when `~/.dsh/profiles/node_modules/@deepseek-ai/<pkg>` is a real
/// directory instead of the managed symlink dsh expects there — observed in
/// practice from a `~/.dsh` a concurrent dsh process had corrupted. See
/// `extract_stale_symlink_path`.
const STALE_SYMLINK_MARKER: &str = " exists and is not a symlink; remove it so dsh can manage the installation fallback";
/// Cap on automatic heal-and-retry cycles per `start_inner` call. dsh reports
/// these one at a time (fixing one reveals the next), so a real `~/.dsh` with
/// several needs more than one retry — but this must stay bounded in case a
/// heal ever fails to actually clear the condition.
const MAX_HEAL_ATTEMPTS: u32 = 5;

/// Optional persistent log file, set once via [`init_log_file`] at startup.
static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Point the persistent log at a file (called once at app setup).
pub fn init_log_file(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = LOG_FILE.set(path);
}

fn append_log_file(line: &str) {
    if let Some(path) = LOG_FILE.get() {
        use std::io::Write as _;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ServerStatus {
    Idle,
    Starting { detail: String },
    Running { url: String },
    Stopped { code: Option<i32>, message: Option<String> },
    Error { message: String },
}

pub struct DshServer {
    pub status: ServerStatus,
    pub logs: VecDeque<String>,
    pid: Option<u32>,
    install_pid: Option<u32>,
    /// Set while a `dsh plugin add` child is running — see `install_plugin`.
    /// Distinct from `install_pid` (the one-time runtime bootstrap): this
    /// one can fire repeatedly across a session, once per plugin install.
    plugin_install_pid: Option<u32>,
    /// Set while an `npm install -g pnpm` child is running — see
    /// `install_pnpm`. A separate lock from `plugin_install_pid`: installing
    /// pnpm and installing a plugin are different resources (global npm
    /// state vs. one dsh web-profile add) and aren't meant to block each
    /// other, even though a plugin install will itself fail without pnpm.
    pnpm_install_pid: Option<u32>,
    requested_stop: bool,
    last_auto_restart: Option<Instant>,
    node: Option<String>,
    bin: Option<String>,
}

impl Default for DshServer {
    fn default() -> Self {
        Self {
            status: ServerStatus::Idle,
            logs: VecDeque::new(),
            pid: None,
            install_pid: None,
            plugin_install_pid: None,
            pnpm_install_pid: None,
            requested_stop: false,
            last_auto_restart: None,
            node: None,
            bin: None,
        }
    }
}

pub type Shared = Arc<Mutex<DshServer>>;

// ── status / logs ────────────────────────────────────────────────────────────

/// Tooltip text for the tray icon — the one status signal visible without
/// opening the window. Kept short (tray tooltips are single-line on most
/// platforms) rather than reusing the boot page's more detailed messages.
fn tray_tooltip_text(status: &ServerStatus) -> String {
    let detail = match status {
        ServerStatus::Idle => "空闲",
        ServerStatus::Starting { .. } => "启动中…",
        ServerStatus::Running { .. } => "运行中",
        ServerStatus::Stopped { .. } => "已停止",
        ServerStatus::Error { .. } => "⚠ 出错",
    };
    format!("DeepSeek Harness — {detail}")
}

fn set_status(app: &AppHandle, server: &Shared, status: ServerStatus) {
    {
        let mut s = server.lock().unwrap();
        s.status = status.clone();
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tray_tooltip_text(&status)));
    }
    let _ = app.emit("server-status", status);
}

fn push_log(server: &Shared, line: String) {
    let mut s = server.lock().unwrap();
    s.logs.push_back(line.clone());
    while s.logs.len() > LOG_CAP {
        s.logs.pop_front();
    }
    drop(s);
    append_log_file(&line);
}

pub fn log_tail(server: &Shared, n: usize) -> Vec<String> {
    let s = server.lock().unwrap();
    let n = n.min(s.logs.len());
    s.logs.iter().skip(s.logs.len() - n).cloned().collect()
}

pub fn running_url(server: &Shared) -> Option<String> {
    match &server.lock().unwrap().status {
        ServerStatus::Running { url } => Some(url.clone()),
        _ => None,
    }
}

// ── paths & environment ──────────────────────────────────────────────────────

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Tauri's `PathResolver` returns verbatim (`\\?\C:\...`) paths on Windows.
/// Node.js chokes on those for module resolution, so convert to the plain
/// `C:\...` form before passing anything to a child process. `pub(crate)`:
/// `panel.rs` reuses this for the `git` subprocess it shells out to.
pub(crate) fn plain_win_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

fn resolve_home(app: &AppHandle) -> PathBuf {
    app.path().home_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// The harness data root: `$DSH_HOME` (with `~/` expansion) or `~/.dsh`.
pub fn dsh_home_dir(app: &AppHandle) -> PathBuf {
    let home = resolve_home(app);
    if let Some(h) = env_nonempty("DSH_HOME") {
        if h == "~" {
            return home;
        }
        if let Some(rest) = h.strip_prefix("~/").or_else(|| h.strip_prefix("~\\")) {
            return home.join(rest);
        }
        return PathBuf::from(h);
    }
    home.join(".dsh")
}

/// Where the managed `@deepseek-ai/dsh` runtime lives (per-user, cache dir).
fn runtime_dir(app: &AppHandle) -> PathBuf {
    if let Some(dir) = env_nonempty("DSH_DESKTOP_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    let cache = app
        .path()
        .app_cache_dir()
        .unwrap_or_else(|_| resolve_home(app));
    cache.join("runtime")
}

fn runtime_bin_path(rd: &Path) -> PathBuf {
    rd.join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js")
}

// ── platform ─────────────────────────────────────────────────────────────────
//
// This file is Windows-only in practice today (the only shipped target is
// NSIS), but the OS-specific bits are isolated here so a future macOS/Linux
// target only needs to extend this section, not chase call sites.
//
// NOTE: the non-Windows branches below are untested — there is no macOS/Linux
// CI or bundled runtime yet. They exist so the crate stays platform-generic;
// treat them as a starting point, not a verified port.

/// The bundled/managed Node executable's filename for this OS.
fn node_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "node.exe"
    } else {
        "node"
    }
}

/// A `<program> …` command that works from a non-interactive spawn context,
/// for PATH-resolved npm-ecosystem CLIs specifically (npm, pnpm, …). On
/// Windows these install as a `.cmd`/`.ps1` shim, not a bare `.exe` —
/// `Command::new(program)` calls `CreateProcess` directly and, unlike a
/// human typing at a shell, never consults `PATHEXT` to try `.cmd`/`.bat`
/// suffixes on a no-extension name, so it fails outright with "program not
/// found" even though the same name works fine typed into any shell. Other
/// platforms invoke the binary directly — no shim layer there.
fn npm_ecosystem_command(program: &str, args: &[&str]) -> Command {
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(program).args(args);
        hide_console(&mut cmd);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

/// A `npm install …` command — see `npm_ecosystem_command`'s own doc
/// comment for why this can't just be `Command::new("npm")` on Windows.
fn npm_install_command(args: &[&str]) -> Command {
    npm_ecosystem_command("npm", args)
}

/// Make `cmd`'s child its own process-group leader (Unix only, no-op call
/// site on Windows). Pairs with [`kill_process_tree`]: killing the negated
/// pid (the group) reaches every descendant without a manual tree walk.
/// Every spawn site that `kill_process_tree` may later target must call this.
#[cfg(unix)]
fn own_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}
#[cfg(not(unix))]
fn own_process_group(_cmd: &mut Command) {}

/// Windows' CREATE_NO_WINDOW flag (winbase.h) — kept as a raw constant rather
/// than pulling in the `windows` crate for one value. `node.exe`, `cmd.exe`
/// and `npm.cmd` are all console-subsystem binaries: spawned from this app's
/// GUI-subsystem process (see `#![windows_subsystem = "windows"]` in
/// main.rs, release builds only) they'd otherwise each pop their own new
/// console window. No-op on other platforms, which have no such concept.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
pub(crate) fn hide_console(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
pub(crate) fn hide_console(_cmd: &mut Command) {}

/// Kill a process and its full descendant tree. Windows uses `taskkill /T
/// /F`, which walks the tree via the OS's own parent-process tracking and
/// kills unconditionally. Other platforms kill the process group instead —
/// see [`own_process_group`], which every spawn site `kill_process_tree` may
/// target must call — but send `SIGTERM`, not `SIGKILL`: a well-behaved
/// child can catch and delay on it, unlike the Windows `/F` path. If that
/// turns out to matter in practice (e.g. a slow shutdown on stop/restart), a
/// short grace period followed by `SIGKILL` on the same group is the fix.
///
/// `pub(crate)`: also the terminal dock card's own close/restart/quit path
/// (terminal.rs) — a `Child::kill()` there would only signal the shell
/// itself, not whatever it's running (an npm/python/nested-shell process
/// the user started interactively), leaving those orphaned.
pub(crate) fn kill_process_tree(pid: u32) {
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        hide_console(&mut cmd);
        let _ = cmd.status();
    } else {
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{pid}")])
            .status();
    }
}

// ── resolution ───────────────────────────────────────────────────────────────

fn resolve_node(app: &AppHandle) -> Result<String, String> {
    if let Some(node) = env_nonempty("DSH_DESKTOP_NODE") {
        if Path::new(&node).exists() {
            return Ok(plain_win_path(Path::new(&node)));
        }
        return Err(format!("DSH_DESKTOP_NODE 指向的文件不存在: {node}"));
    }
    // bundled runtime shipped with packaged builds
    if let Ok(res) = app.path().resource_dir() {
        let bundled = res.join("runtime").join(node_binary_name());
        if bundled.exists() {
            return Ok(plain_win_path(&bundled));
        }
    }
    // system node from PATH
    let mut probe = Command::new("node");
    probe.arg("--version");
    hide_console(&mut probe);
    match probe.output() {
        Ok(out) if out.status.success() => Ok("node".to_string()),
        _ => Err(
            "未检测到 Node.js：请安装 Node.js（https://nodejs.org），\
             或在环境变量 DSH_DESKTOP_NODE 中指定 node.exe 的路径。"
                .to_string(),
        ),
    }
}

fn resolve_bin(app: &AppHandle, server: &Shared) -> Result<String, String> {
    if let Some(bin) = env_nonempty("DSH_DESKTOP_DSH_BIN") {
        let p = PathBuf::from(&bin);
        if p.exists() {
            push_log(server, format!("使用 DSH_DESKTOP_DSH_BIN: {bin}"));
            return Ok(plain_win_path(&p));
        }
        return Err(format!("DSH_DESKTOP_DSH_BIN 指向的文件不存在: {bin}"));
    }
    // bundled runtime shipped with packaged builds
    if let Ok(res) = app.path().resource_dir() {
        let bundled = runtime_bin_path(&res.join("runtime"));
        if bundled.exists() {
            return Ok(plain_win_path(&bundled));
        }
    }
    let rd = runtime_dir(app);
    let bin = runtime_bin_path(&rd);
    if !bin.exists() {
        install_runtime(app, server, &rd)?;
    }
    if !bin.exists() {
        return Err(format!("dsh 运行时安装失败（{}），请查看日志。", rd.display()));
    }
    Ok(plain_win_path(&bin))
}

fn install_runtime(app: &AppHandle, server: &Shared, rd: &Path) -> Result<(), String> {
    let version = env_nonempty("DSH_DESKTOP_DSH_VERSION").unwrap_or_else(|| DSH_VERSION_DEFAULT.to_string());
    let target = format!("@deepseek-ai/dsh@{version}");
    fs::create_dir_all(rd).map_err(|e| format!("无法创建运行时目录 {}: {e}", rd.display()))?;
    push_log(server, format!("首次使用：安装 dsh 运行时 ({target}) → {}", rd.display()));
    set_status(
        app,
        server,
        ServerStatus::Starting {
            detail: format!("安装 dsh 运行时 ({version})…"),
        },
    );
    let prefix = plain_win_path(rd);
    let mut cmd = npm_install_command(&[
        "install",
        "--prefix",
        &prefix,
        &target,
        "--omit=dev",
        "--no-audit",
        "--no-fund",
        "--no-progress",
        "--prefer-offline",
        "--fetch-retries=5",
        "--fetch-retry-mintimeout=2000",
    ]);
    own_process_group(&mut cmd);
    let mut output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法运行 npm install: {e}"))?;

    // Stream npm output into the log ring buffer so first-run installs
    // (hundreds of MB) are visible instead of silent.
    let install_pid = output.id();
    {
        let mut s = server.lock().unwrap();
        s.install_pid = Some(install_pid);
    }
    let stdout = output.stdout.take().expect("stdout pipe");
    let stderr = output.stderr.take().expect("stderr pipe");
    let s1 = server.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            push_log(&s1, line);
        }
    });
    let s2 = server.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            push_log(&s2, line);
        }
    });
    // Poll wait so `stop()` (which taskkills `install_pid`) is honoured even
    // while the install is running.
    let status = loop {
        match output.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(300)),
            Err(e) => {
                // Clear install_pid before returning — otherwise a stale PID
                // lingers in shared state and a later `stop()` could
                // taskkill whatever unrelated process the OS has since
                // reused that PID number for.
                server.lock().unwrap().install_pid = None;
                return Err(format!("npm install 进程错误: {e}"));
            }
        }
    };
    {
        let mut s = server.lock().unwrap();
        s.install_pid = None;
    }
    if !status.success() {
        return Err(format!("npm install 失败（exit {:?}），请查看日志。", status.code()));
    }
    Ok(())
}

// ── plugin install ───────────────────────────────────────────────────────────
//
// Thin wrapper around `dsh plugin --profile web add <package>` — verified
// against the real bundled `dsh` binary's own --help output to actually
// forward to `pnpm` inside the web profile's directory (a real pnpm
// workspace under `$DSH_HOME/profiles/web`), not a bespoke installer this
// project would need to maintain. `package` is never sourced from the
// harness page (zero IPC — see lib.rs's module doc comment): it only ever
// reaches here via this shell's own UI. The UI's catalog itself (see
// PLUGIN_CATALOG_URL in app.js) is fetched from a crowd-submitted registry
// this project doesn't vet — this function has no opinion on that, and
// isn't the safety boundary for it; the confirm-before-install step in the
// plugin market dialog is.

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum PluginInstallEvent {
    Line { text: String },
    Done { success: bool, code: Option<i32> },
}

/// Runs `dsh plugin --profile web add <package>`, streaming stdout/stderr
/// lines to the `plugin-install` event as they arrive (installs can take a
/// while — pnpm resolving/downloading a package tree — so the UI needs
/// progress, not just a final result). Emits one `Done` event when the
/// child exits, success or not.
///
/// Single-flight: rejects a call while an earlier install is still running,
/// rather than letting two concurrent `pnpm add` children interleave their
/// output into the same `plugin-install` stream with no way for a listener
/// to tell which line belongs to which. The frontend's own per-button
/// disabled state is a UX nicety on top of this, not a substitute for it —
/// this guard is what actually makes it safe.
pub fn install_plugin(app: &AppHandle, server: &Shared, package: &str) -> Result<(), String> {
    {
        let mut s = server.lock().unwrap();
        if s.plugin_install_pid.is_some() {
            return Err("已有插件正在安装，请等待完成后再试。".to_string());
        }
        // Reserved with a placeholder before resolve_node/resolve_bin (which
        // can themselves install the runtime and take a while) so a second
        // call arriving during that window is still rejected, not raced.
        s.plugin_install_pid = Some(0);
    }

    let node = match resolve_node(app) {
        Ok(v) => v,
        Err(e) => {
            server.lock().unwrap().plugin_install_pid = None;
            return Err(e);
        }
    };
    let bin = match resolve_bin(app, server) {
        Ok(v) => v,
        Err(e) => {
            server.lock().unwrap().plugin_install_pid = None;
            return Err(e);
        }
    };

    let mut cmd = Command::new(&node);
    cmd.arg(&bin).arg("plugin").arg("--profile").arg("web").arg("add").arg(package);
    // Same PATH rebuild as the server process itself — pnpm needs a real
    // PATH to find node/git, not whatever this GUI process happened to
    // inherit at launch. See `effective_path`'s own doc comment.
    cmd.env("PATH", effective_path());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    own_process_group(&mut cmd);
    hide_console(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            server.lock().unwrap().plugin_install_pid = None;
            return Err(format!("无法运行 dsh plugin add: {e}"));
        }
    };

    server.lock().unwrap().plugin_install_pid = Some(child.id());

    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let app1 = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = app1.emit("plugin-install", PluginInstallEvent::Line { text: line });
        }
    });
    let app2 = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = app2.emit("plugin-install", PluginInstallEvent::Line { text: line });
        }
    });

    let app3 = app.clone();
    let srv = server.clone();
    thread::spawn(move || {
        let status = child.wait();
        let (success, code) = match status {
            Ok(st) => (st.success(), st.code()),
            Err(_) => (false, None),
        };
        srv.lock().unwrap().plugin_install_pid = None;
        let _ = app3.emit("plugin-install", PluginInstallEvent::Done { success, code });
    });

    Ok(())
}

// ── pnpm detection & one-click install ──────────────────────────────────────
//
// `dsh plugin add` (install_plugin above) shells out to `pnpm` inside the
// web profile directory — if pnpm isn't on PATH, that fails with a raw OS
// "not found" error the plugin market's confirm view has no way to explain.
// check_pnpm_available lets the frontend probe for that *before* offering
// "确认安装", and install_pnpm (`npm install -g pnpm`) is the fix it can
// offer inline instead of sending the user to a terminal.

/// Same probe style as `resolve_node`'s system-node check: shell out with
/// `--version` and see if it succeeds. Goes through `npm_ecosystem_command`
/// (not a bare `Command::new("pnpm")`) since pnpm installs as a `.cmd` shim
/// on Windows too — the same gap that made `install_pnpm`'s own first cut
/// fail outright with "program not found" would otherwise make this probe
/// permanently report "unavailable" even right after a successful install.
/// Uses `effective_path()` — the same rebuilt PATH `install_plugin` itself
/// runs pnpm under — so this reports what a real install attempt would
/// actually see, not just whatever PATH this GUI process happened to
/// inherit at launch.
pub fn check_pnpm_available() -> bool {
    let mut probe = npm_ecosystem_command("pnpm", &["--version"]);
    probe.env("PATH", effective_path());
    probe.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    matches!(probe.status(), Ok(status) if status.success())
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum PnpmInstallEvent {
    Line { text: String },
    Done { success: bool, code: Option<i32> },
}

/// Runs `npm install -g pnpm`, streaming output to the `pnpm-install` event —
/// same shape as `install_plugin`'s own streaming/single-flight design, on
/// its own lock (`pnpm_install_pid`) since this installs into the user's
/// global npm state, a different resource than a single dsh plugin add.
/// Needs a real system `npm`, not the bundled runtime (that runtime only
/// ships `node` + the vendored `dsh` package tree, no global npm install
/// target of its own) — resolve_node's PATH fallback covers this the same
/// way it covers "no bundled runtime at all" for node itself. Goes through
/// `npm_ecosystem_command`, not `Command::new("npm")` directly — npm is a
/// `.cmd` shim on Windows (see that function's own doc comment), so the
/// bare form spawned fine on other platforms but failed outright here with
/// a raw "program not found" the confirm view had no way to explain.
pub fn install_pnpm(app: &AppHandle, server: &Shared) -> Result<(), String> {
    {
        let mut s = server.lock().unwrap();
        if s.pnpm_install_pid.is_some() {
            return Err("pnpm 正在安装，请等待完成后再试。".to_string());
        }
        s.pnpm_install_pid = Some(0);
    }

    let mut cmd = npm_ecosystem_command("npm", &["install", "-g", "pnpm"]);
    cmd.env("PATH", effective_path());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    own_process_group(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            server.lock().unwrap().pnpm_install_pid = None;
            return Err(format!(
                "无法运行 npm install -g pnpm: {e}（请确认系统已安装 Node.js/npm）"
            ));
        }
    };

    server.lock().unwrap().pnpm_install_pid = Some(child.id());

    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let app1 = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = app1.emit("pnpm-install", PnpmInstallEvent::Line { text: line });
        }
    });
    let app2 = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = app2.emit("pnpm-install", PnpmInstallEvent::Line { text: line });
        }
    });

    let app3 = app.clone();
    let srv = server.clone();
    thread::spawn(move || {
        let status = child.wait();
        let (success, code) = match status {
            Ok(st) => (st.success(), st.code()),
            Err(_) => (false, None),
        };
        srv.lock().unwrap().pnpm_install_pid = None;
        let _ = app3.emit("pnpm-install", PnpmInstallEvent::Done { success, code });
    });

    Ok(())
}

// ── self-heal: stale profiles/node_modules symlinks ─────────────────────────

/// Extracts the offending path from a dsh bootstrap error line matching
/// [`STALE_SYMLINK_MARKER`], e.g.:
/// `dsh: C:\Users\x\.dsh\profiles\node_modules\@deepseek-ai\foo exists and
/// is not a symlink; remove it so dsh can manage the installation fallback`
/// Returns `None` for any line that doesn't match this exact, narrow shape.
fn extract_stale_symlink_path(line: &str) -> Option<PathBuf> {
    let marker_idx = line.find(STALE_SYMLINK_MARKER)?;
    let prefix = "dsh: ";
    let prefix_idx = line.find(prefix)?;
    if prefix_idx >= marker_idx {
        return None;
    }
    let path_str = line[prefix_idx + prefix.len()..marker_idx].trim();
    if path_str.is_empty() {
        return None;
    }
    Some(PathBuf::from(path_str))
}

/// Removes a stale, empty, non-symlink directory that dsh's own bootstrap
/// flagged (see `extract_stale_symlink_path`) so dsh can recreate it as the
/// managed symlink it expects on the next attempt. Deliberately narrow:
/// only acts on a path that is (a) inside `~/.dsh/profiles/node_modules/
/// @deepseek-ai/`, matching every real instance seen so far, and (b) an
/// empty directory, not a symlink — anything else is left alone (and logged
/// as skipped) rather than silently deleted on dsh's stderr's say-so alone.
/// Returns whether it actually removed something.
fn heal_stale_symlink(app: &AppHandle, server: &Shared, path: &Path) -> bool {
    let expected_prefix = dsh_home_dir(app).join("profiles").join("node_modules").join("@deepseek-ai");
    if !path.starts_with(&expected_prefix) {
        push_log(server, format!("跳过自动清理（路径不在预期范围内）：{}", path.display()));
        return false;
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return false, // already gone (e.g. a stale re-detection of an earlier heal)
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        push_log(server, format!("跳过自动清理（不是预期的普通目录）：{}", path.display()));
        return false;
    }
    let is_empty = fs::read_dir(path).map(|mut it| it.next().is_none()).unwrap_or(false);
    if !is_empty {
        push_log(server, format!("跳过自动清理（目录非空，可能有需要保留的内容）：{}", path.display()));
        return false;
    }
    match fs::remove_dir(path) {
        Ok(()) => {
            push_log(
                server,
                format!("已自动清理残留目录（dsh 将在下次启动时重建为符号链接）：{}", path.display()),
            );
            true
        }
        Err(e) => {
            push_log(server, format!("自动清理残留目录失败：{} ({e})", path.display()));
            false
        }
    }
}

// ── PATH ─────────────────────────────────────────────────────────────────────

/// A GUI app launched from Explorer/Start Menu inherits whatever `PATH` its
/// own parent process had — normally fine, but that value is only ever
/// recomputed at logon. Install Node/Git/etc. and *not* log off first (a
/// completely ordinary sequence: install desktop app → open it right away)
/// and the running desktop process — and everything it spawns, including
/// every tool `dsh` shells out to on the agent's behalf (pwsh, git, …) —
/// inherits that stale, possibly near-empty `PATH`. Rebuild it from the
/// registry (the actual source of truth Windows itself recomputes PATH
/// from at logon) instead of trusting whatever this process happened to
/// start with. Falls back to the process's own `PATH` if the registry read
/// fails for any reason — never worse than doing nothing.
/// Registry `PATH` values are commonly `REG_EXPAND_SZ` — a template string
/// with literal `%VAR%` placeholders (this machine's own machine-level
/// PATH includes `%NVM_SYMLINK%`, `%JAVA_HOME%\bin`, `%SystemRoot%`, …) —
/// but `winreg`'s own `get_value::<String, _>` decodes `REG_EXPAND_SZ`
/// exactly like plain `REG_SZ`: no expansion, the `%...%` comes through
/// verbatim (see winreg's types.rs, where both variants hit the same
/// string-decode branch). A tool installed under an expanded path (pnpm
/// under nvm4w's `%NVM_SYMLINK%\nodejs`, say) then never resolves: the
/// PATH entry Windows itself would expand at logon stays a literal,
/// unmatchable string in every child process this app spawns. Win32's own
/// `ExpandEnvironmentStringsW` is the standard way to do what Windows does
/// automatically for interactively-launched processes; there's no
/// winreg-level equivalent to reach for instead. Two-call convention: pass
/// `None` first to get the required buffer length, then a real buffer —
/// safe here because `dst` never has an interior NUL for the API to choke
/// on (Rust `String`s already reject one) and `HSTRING::from` produces the
/// NUL-terminated UTF-16 the first call's *including-NUL* length assumes.
#[cfg(windows)]
fn expand_env_string(raw: &str) -> String {
    use windows::core::HSTRING;
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;

    let src = HSTRING::from(raw);
    // SAFETY: `lpdst: None` just asks for the required length — no buffer
    // is written, so there's nothing for the FFI call to write out of
    // bounds of.
    let needed = unsafe { ExpandEnvironmentStringsW(&src, None) };
    if needed == 0 {
        // Expansion failed (e.g. an unresolvable %VAR%) — the raw,
        // unexpanded string is still a usable PATH segment on its own
        // (Windows itself falls back to leaving unmatched %VAR% literal
        // rather than erroring), so degrade to that instead of dropping it.
        return raw.to_string();
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: `buf` is sized to exactly the length the first call reported
    // as required (including the NUL terminator), so the second call's
    // write stays within `buf`'s bounds.
    let written = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) };
    if written == 0 || written as usize > buf.len() {
        return raw.to_string();
    }
    // `written` counts the NUL terminator; trim it before decoding so the
    // result doesn't carry an embedded U+0000 into the PATH string this
    // feeds into.
    String::from_utf16_lossy(&buf[..written as usize - 1])
}

#[cfg(windows)]
fn effective_path() -> String {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
        .and_then(|k| k.get_value::<String, _>("Path"))
        .map(|p| expand_env_string(&p))
        .unwrap_or_default();
    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .and_then(|k| k.get_value::<String, _>("Path"))
        .map(|p| expand_env_string(&p))
        .unwrap_or_default();
    // Matches Windows' own logon-time order: machine entries first, then
    // user entries. Also keep whatever this process already had (e.g. a
    // launcher-prepended entry) so nothing already-working regresses.
    let current = env_nonempty("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = Vec::new();
    for source in [machine.as_str(), user.as_str(), current.as_str()] {
        for p in source.split(';') {
            let p = p.trim();
            if !p.is_empty() && !parts.contains(&p) {
                parts.push(p);
            }
        }
    }
    parts.join(";")
}
#[cfg(not(windows))]
fn effective_path() -> String {
    env_nonempty("PATH").unwrap_or_default()
}

// ── probing & spawning ───────────────────────────────────────────────────────

/// Cheap dependency-free HTTP probe: does `http://host:port/` serve the
/// harness index page?
fn probe_dsh(url: &str) -> bool {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(80)),
        None => (rest, 80),
    };
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let Some(sockaddr) = addrs.next() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&sockaddr, Duration::from_millis(1200)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}:{port}\r\nAccept-Encoding: identity\r\nConnection: close\r\nUser-Agent: dsh-desktop\r\n\r\n"
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > 131072 {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).contains(INDEX_MARKER)
}

fn set_running(app: &AppHandle, server: &Shared, url: &str) {
    println!("[dsh-desktop] server running at {url}");
    {
        let mut s = server.lock().unwrap();
        s.status = ServerStatus::Running {
            url: url.to_string(),
        };
    }
    let _ = app.emit(
        "server-status",
        ServerStatus::Running {
            url: url.to_string(),
        },
    );
    // The container page (ui/) never navigates away — it points an <iframe>
    // at this URL itself, in reaction to the event just emitted above. See
    // the iframe-based layout note on `DshServer` below for why this
    // replaced a `win.navigate()` call to the harness URL.
}

/// Marks the server stopped and notifies the container page. Used by the
/// `stop_server` command: previously, `stop()` alone left `status` stale
/// until an exit-watcher thread eventually reaped the killed process — that
/// staleness was masked by a forced `win.navigate()` back to the boot page,
/// which reloaded fresh JS that would soon re-poll and catch up. Now that
/// the container page is never reloaded (it hosts the harness in an
/// `<iframe>` instead of navigating its own top level), that reload no
/// longer happens to paper over the gap, so this sets the real status
/// directly instead.
pub fn set_stopped(app: &AppHandle, server: &Shared) {
    set_status(app, server, ServerStatus::Stopped { code: None, message: None });
}

/// The workspace directory the server — and this shell's own
/// workspace-aware features (the file/git panel in `panel.rs`) — operate on:
/// `DSH_DESKTOP_CWD` if set, else the user's home directory.
pub(crate) fn workspace_dir(app: &AppHandle) -> PathBuf {
    env_nonempty("DSH_DESKTOP_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_home(app))
}

/// Spawn the server process and start its reader/exit-watcher threads.
fn spawn(app: &AppHandle, server: &Shared, node: &str, bin: &str, port: u16) -> Result<(), String> {
    let cwd = workspace_dir(app);
    let cwd_plain = plain_win_path(&cwd);
    fs::create_dir_all(&cwd).map_err(|e| format!("无法创建工作目录 {}: {e}", cwd.display()))?;

    push_log(
        server,
        format!("启动: {node} {bin} web --port {port}  (cwd: {cwd_plain})"),
    );

    let mut cmd = Command::new(node);
    cmd.arg(bin).arg("web").arg("--port").arg(port.to_string());
    cmd.current_dir(&cwd_plain);
    // See `effective_path`: every tool call dsh shells out to on the
    // agent's behalf inherits this, so a stale/truncated PATH here doesn't
    // just affect the dsh process — it breaks pwsh/git/etc. tool calls too.
    cmd.env("PATH", effective_path());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    own_process_group(&mut cmd);
    hide_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("启动 dsh 失败: {e}"))?;

    let pid = child.id();
    println!("[dsh-desktop] spawned dsh pid {pid} on port {port}");
    {
        let mut s = server.lock().unwrap();
        s.pid = Some(pid);
        s.requested_stop = false;
        s.node = Some(node.to_string());
        s.bin = Some(bin.to_string());
    }
    set_status(
        app,
        server,
        ServerStatus::Starting {
            detail: format!("服务启动中 (端口 {port})…"),
        },
    );

    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");

    // stdout reader: capture logs and discover the printed URL line
    let app2 = app.clone();
    let srv2 = server.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            push_log(&srv2, line.clone());
            if let Some(idx) = line.find(URL_PREFIX) {
                let rest = &line[idx + URL_PREFIX.len()..];
                let port: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !port.is_empty() {
                    // Only this reader's own child may report itself ready. A
                    // stale line from a child already superseded by a newer
                    // spawn (e.g. a fast restart) must not resurrect it as
                    // the active server — see the exit watcher below for the
                    // same guard against the mirror-image race.
                    let is_current = srv2.lock().unwrap().pid == Some(pid);
                    if is_current {
                        let url = format!("http://127.0.0.1:{port}");
                        set_running(&app2, &srv2, &url);
                    }
                }
            }
        }
    });

    // stderr reader
    let srv3 = server.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            push_log(&srv3, line);
        }
    });

    // exit watcher: auto-restart once per 60s window, then surface the error
    let app3 = app.clone();
    let srv4 = server.clone();
    thread::spawn(move || {
        let exit = child.wait();
        let code = exit.ok().and_then(|st| st.code());
        // If a newer spawn already replaced this child (e.g. `restart()`
        // killed it and started a fresh one before this thread got to reap
        // the old one), `s.pid` now holds that *other*, live pid — not `pid`
        // and not `None`. Only that specific case is stale: `s.pid == None`
        // still means "nobody's spawned since `stop()` cleared it", which is
        // the ordinary stop path and must still fall through to report
        // `Stopped` below. Don't clobber a newer pid; do clear our own.
        let (requested, is_stale) = {
            let mut s = srv4.lock().unwrap();
            let is_stale = matches!(s.pid, Some(other) if other != pid);
            if !is_stale {
                s.pid = None;
            }
            (s.requested_stop, is_stale)
        };
        if is_stale {
            return;
        }
        if requested {
            set_status(&app3, &srv4, ServerStatus::Stopped { code, message: None });
            return;
        }
        push_log(&srv4, format!("dsh 服务退出 (exit {code:?})"));

        let allow_restart = {
            let s = srv4.lock().unwrap();
            match s.last_auto_restart {
                Some(t) => t.elapsed() >= AUTO_RESTART_MIN_GAP,
                None => true,
            }
        };
        if allow_restart {
            {
                let mut s = srv4.lock().unwrap();
                s.last_auto_restart = Some(Instant::now());
            }
            set_status(
                &app3,
                &srv4,
                ServerStatus::Error {
                    message: format!("dsh 服务异常退出 (exit {code:?})，1 秒后自动重启…"),
                },
            );
            thread::sleep(Duration::from_secs(1));
            let _ = start(&app3, &srv4);
        } else {
            let status = ServerStatus::Error {
                message: format!("dsh 服务异常退出 (exit {code:?})，自动重启已停止，请手动重试。"),
            };
            {
                let mut s = srv4.lock().unwrap();
                s.status = status.clone();
            }
            let _ = app3.emit("server-status", status);
        }
    });

    Ok(())
}

// ── public API ───────────────────────────────────────────────────────────────

/// Stop the server: mark requested, then kill the whole process tree.
pub fn stop(server: &Shared) {
    let (pid, install_pid) = {
        let mut s = server.lock().unwrap();
        s.requested_stop = true;
        (s.pid.take(), s.install_pid.take())
    };
    if let Some(pid) = pid {
        println!("[dsh-desktop] stopping dsh pid {pid} (process tree)");
        kill_process_tree(pid);
    }
    if let Some(pid) = install_pid {
        println!("[dsh-desktop] stopping runtime install pid {pid}");
        kill_process_tree(pid);
    }
}

/// Stop the current server, then start fresh. Blocks briefly (this is meant
/// to be called from a spawned thread, not the main/event-loop thread) so
/// the old process tree is well clear of the port/`~/.dsh` before the new
/// one starts. Shared by the tray/menu restart action and the
/// `restart_server` command so both go through the exact same choreography.
pub fn restart(app: &AppHandle, server: &Shared) {
    stop(server);
    thread::sleep(Duration::from_millis(600));
    let _ = start(app, server);
}

/// The default bind port; `DSH_DESKTOP_PORT` overrides it (useful for
/// running several desktop instances side by side for testing).
pub fn default_port() -> u16 {
    env_nonempty("DSH_DESKTOP_PORT")
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(DEFAULT_PORT)
}

/// Start (or attach to) the harness server. Never blocks for long: spawning
/// and URL discovery happen on worker threads started inside. Failures are
/// surfaced as `ServerStatus::Error` so the boot page always shows a reason.
pub fn start(app: &AppHandle, server: &Shared) -> Result<(), String> {
    {
        let s = server.lock().unwrap();
        if matches!(s.status, ServerStatus::Running { .. } | ServerStatus::Starting { .. }) {
            return Ok(());
        }
    }

    let result = start_inner(app, server);
    if let Err(msg) = &result {
        push_log(server, format!("启动失败: {msg}"));
        set_status(app, server, ServerStatus::Error { message: msg.clone() });
    }
    result
}

fn start_inner(app: &AppHandle, server: &Shared) -> Result<(), String> {
    // Attach to an already-running harness on the default port instead of
    // spawning a second instance (two servers would race on ~/.dsh). This
    // happens before resolving node/runtime so attach mode needs nothing.
    let port = default_port();
    let default_url = format!("http://127.0.0.1:{port}");
    if probe_dsh(&default_url) {
        push_log(server, format!("检测到已在运行的 dsh 服务，直接使用 {default_url}"));
        set_running(app, server, &default_url);
        return Ok(());
    }

    let node = resolve_node(app)?;
    let bin = resolve_bin(app, server)?;
    push_log(server, format!("node: {node}"));
    push_log(server, format!("bin:  {bin}"));

    spawn(app, server, &node, &bin, port)?;

    // If the port is taken by a non-dsh process, the child exits quickly with
    // EADDRINUSE; fall back to an OS-assigned port (the URL line is parsed
    // from stdout by the reader thread). If it exits because dsh's own
    // bootstrap found a stale, non-symlink entry under ~/.dsh/profiles (see
    // `heal_stale_symlink`), clear it and retry on the same port — bounded,
    // since dsh reports these one at a time and a real ~/.dsh can have more
    // than one. If the process stays alive but never prints its ready URL
    // line — e.g. an upstream format change broke `URL_PREFIX` matching —
    // surface an error instead of leaving the boot page stuck on "starting"
    // forever.
    let ready_deadline = Instant::now() + READY_TIMEOUT;
    let mut heal_attempts = 0u32;
    loop {
        let (running, exited, eaddr, stale_symlink) = {
            let s = server.lock().unwrap();
            let stale_symlink = s.logs.iter().rev().find_map(|l| extract_stale_symlink_path(l));
            (
                matches!(s.status, ServerStatus::Running { .. }),
                s.pid.is_none(),
                s.logs.iter().any(|l| l.contains("EADDRINUSE")),
                stale_symlink,
            )
        };
        if running {
            return Ok(());
        }
        if exited && eaddr {
            push_log(server, "端口 3080 被其他程序占用，改用系统分配端口…".to_string());
            spawn(app, server, &node, &bin, 0)?;
            return Ok(());
        }
        if exited {
            if let Some(path) = &stale_symlink {
                if heal_attempts < MAX_HEAL_ATTEMPTS && heal_stale_symlink(app, server, path) {
                    heal_attempts += 1;
                    spawn(app, server, &node, &bin, port)?;
                    continue;
                }
            }
            // Exited for a reason other than the above: the exit-watcher
            // thread from `spawn` already owns status/logs for this case
            // (auto-restart or a surfaced error), nothing more to do here.
            return Ok(());
        }
        if Instant::now() > ready_deadline {
            push_log(
                server,
                "dsh 服务进程仍在运行，但超时未探测到就绪 URL（可能是 dsh 输出格式发生变化）。".to_string(),
            );
            set_status(
                app,
                server,
                ServerStatus::Error {
                    message: "启动超时：dsh 进程仍在运行，但未能确认服务已就绪，请查看日志。".to_string(),
                },
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Info for the boot page footer.
pub fn info(app: &AppHandle, server: &Shared) -> serde_json::Value {
    let (node, bin) = {
        let s = server.lock().unwrap();
        (s.node.clone(), s.bin.clone())
    };
    let dsh_version = env_nonempty("DSH_DESKTOP_DSH_VERSION").unwrap_or_else(|| DSH_VERSION_DEFAULT.to_string());
    serde_json::json!({
        "dshVersion": dsh_version,
        "nodePath": node.unwrap_or_else(|| "未检测到".to_string()),
        "binPath": bin.unwrap_or_default(),
        "dshHome": dsh_home_dir(app).to_string_lossy(),
        "runtimeDir": runtime_dir(app).to_string_lossy(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real error lines captured verbatim from `@deepseek-ai/dsh-app-boot`'s
    // `ensureSymlink` (two separate incidents, same shape, different package).
    const REAL_LINE_1: &str = r"Error: dsh: C:\Users\him69\.dsh\profiles\node_modules\@deepseek-ai\cordis-plugin-loader exists and is not a symlink; remove it so dsh can manage the installation fallback";
    const REAL_LINE_2: &str = r"Error: dsh: C:\Users\him69\.dsh\profiles\node_modules\@deepseek-ai\dsh-spill-policy exists and is not a symlink; remove it so dsh can manage the installation fallback";

    #[test]
    fn extracts_path_from_real_captured_errors() {
        assert_eq!(
            extract_stale_symlink_path(REAL_LINE_1),
            Some(PathBuf::from(
                r"C:\Users\him69\.dsh\profiles\node_modules\@deepseek-ai\cordis-plugin-loader"
            ))
        );
        assert_eq!(
            extract_stale_symlink_path(REAL_LINE_2),
            Some(PathBuf::from(
                r"C:\Users\him69\.dsh\profiles\node_modules\@deepseek-ai\dsh-spill-policy"
            ))
        );
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert_eq!(extract_stale_symlink_path("dsh web: http://127.0.0.1:3080"), None);
        assert_eq!(extract_stale_symlink_path(""), None);
        assert_eq!(
            extract_stale_symlink_path("some other error entirely unrelated to symlinks"),
            None
        );
    }

    #[test]
    fn ignores_marker_without_a_path_prefix() {
        // Marker present but no "dsh: " prefix before it — malformed/foreign
        // input should not be parsed as a real dsh error.
        assert_eq!(
            extract_stale_symlink_path("exists and is not a symlink; remove it so dsh can manage the installation fallback"),
            None
        );
    }
}
