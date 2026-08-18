# Development guide

[中文](DEVELOPMENT.md) | English

For contributors, people building from source, or anyone curious how the app works internally.
For day-to-day usage, see the [main README](../README.en.md).

## Prerequisites

- [Rust](https://rustup.rs/) (MSVC toolchain) — for the Tauri shell
- [Node.js](https://nodejs.org/) >= 22 — required by `dsh` itself (the app locates it on `PATH`)
- [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (preinstalled on
  Windows 11 / most Windows 10)

## Development

```bash
npm install          # installs @tauri-apps/cli
npm run tauri dev    # builds the Rust shell and opens the app window
```

On first launch the app installs the `@deepseek-ai/dsh` npm package into a per-user runtime
directory (`%LOCALAPPDATA%\dev.dsh.desktop\runtime`) and starts it. The install is cached by npm,
so it is fast and offline after the first run.

### Environment overrides

| Variable | Purpose |
|---|---|
| `DSH_DESKTOP_NODE` | Absolute path to `node.exe` to use instead of the one on `PATH` |
| `DSH_DESKTOP_DSH_BIN` | Absolute path to a `dsh` `lib/bin.js` (e.g. a local checkout) |
| `DSH_DESKTOP_RUNTIME_DIR` | Where the managed `@deepseek-ai/dsh` runtime is installed (default: app cache dir); point it at an existing `node_modules` root to skip the first-run npm install |
| `DSH_DESKTOP_DSH_VERSION` | npm version spec for the managed runtime (default `0.1.0-rc.7`) |
| `DSH_DESKTOP_PORT` | Default bind port override (default `3080`); handy for running several instances |
| `DSH_DESKTOP_CWD` | Working directory for the `dsh` server process (default: user home) |
| `DSH_HOME` | Passed through to the server; harness data root (default `~/.dsh`) |

## Architecture

```
┌─ Tauri app (Rust, WebView2) ─────────────────────────────┐
│ local boot page (loading / error / retry)                │
│   └─ navigates to → http://127.0.0.1:<port> (the UI)     │
│ server manager (src-tauri/src/server.rs)                 │
│   locate node → install/verify dsh runtime → probe 3080  │
│   → spawn `node dsh web --port …` → parse stdout URL     │
│   → navigate → watch process → taskkill tree on exit     │
│ native menu & tray (src-tauri/src/menu.rs)               │
└─────────────────────────┬────────────────────────────────┘
                          │ spawn
                 ┌────────▼────────┐
                 │  dsh web server │  data → ~/.dsh (DSH_HOME)
                 └─────────────────┘
```

The harness page is loaded from `http://127.0.0.1:<port>` and is intentionally **not** granted
Tauri IPC access (`dangerousRemoteDomainIpcAccess` is never enabled), so the web UI cannot reach
the shell — every shell action goes through the native menu/tray or the local boot page.

## Building the installer

```bash
npm install
npm run bundle        # fetch:node → prepare:runtime → tauri build
# or step by step:
npm run fetch:node    # downloads node.exe → src-tauri/resources/runtime
DSH_RUNTIME_SOURCE=<node_modules root> npm run prepare:runtime  # copy a local runtime instead of npm install
npm run build         # → src-tauri/target/release/bundle/nsis/DeepSeek Harness_<version>_x64-setup.exe
```

Before cutting a release, run `npm run check:dsh-version` — upstream is in developer preview and
publishes new RCs without notice; this checks the pinned `@deepseek-ai/dsh` default (duplicated in
`src-tauri/src/server.rs` and `scripts/prepare-runtime.mjs`, they must agree) against npm's latest.
The release workflow runs this same check and fails the build on a mismatch.

### Two version axes

This app has two independent version numbers that must not be conflated:

- **Shell version** (`tauri.conf.json`'s `version`) — the desktop wrapper itself.
  `tauri-plugin-updater` only updates this.
- **Runtime version** (`DSH_VERSION_DEFAULT` in `server.rs` / the default in
  `prepare-runtime.mjs`) — the pinned `@deepseek-ai/dsh` release bundled inside the installer or
  installed on first use.

**For a bundled-runtime install (the default, `npm run bundle`)** these travel together
automatically: the NSIS installer's payload includes `resources/runtime/`, so a shell
auto-update reinstalls the runtime pinned at build time along with it — there's no separate
runtime-update mechanism to build as long as `DSH_VERSION_DEFAULT` is bumped (and
`check:dsh-version` passes) before cutting each shell release.

**For the managed (non-bundled) runtime path** — used when there's no `resources/runtime/`
(e.g. an unpackaged dev build, or `DSH_DESKTOP_RUNTIME_DIR` pointed elsewhere) — the runtime is
installed once via `npm install` on first use ([server.rs](../src-tauri/src/server.rs)'s
`install_runtime`) and **never re-checked afterward**. A user on this path who wants a newer
`dsh` has to clear `DSH_DESKTOP_RUNTIME_DIR` (or set `DSH_DESKTOP_DSH_VERSION` to a newer spec)
and let it reinstall. This is a known, narrow gap — not worth a bespoke updater for a path that's
mainly used in development.
</content>
