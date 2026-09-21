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
| `DSH_DESKTOP_DSH_VERSION` | npm version spec for the managed runtime (default `0.1.5-rc.2`, matching npm's `latest`; override it to try a version that hasn't been promoted yet) |
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
npm run build          # tauri build auto-runs fetch:node + prepare:runtime first, see below
# npm run bundle is the same thing spelled out explicitly — same effect
```

`tauri.conf.json`'s `beforeBuildCommand` wires `fetch:node`/`prepare:runtime` into `tauri build`
itself, so `npm run build`, `npm run tauri build`, and `cargo tauri build` all get it regardless of
how they're invoked. This is deliberate: `src-tauri/resources/runtime` is a gitignored, manually
generated artifact — before this hook existed, running `tauri build` directly (bypassing
`npm run bundle`) would silently package whatever version happened to be sitting on disk, even if
it had drifted from `DSH_VERSION_DEFAULT`. That drift is exactly what crashed a locally-installed
build: its `resources/runtime` was stuck on an old version, nothing had ever forced the two back in
sync, and the user ran straight into a command-line flag the bundled runtime didn't recognize.

If you're substituting a local checkout for the npm registry (`DSH_RUNTIME_SOURCE`), carry that
variable into the build step too, not just the manual `prepare:runtime` run — `beforeBuildCommand`
re-runs `prepare:runtime` right before packaging, and without the variable it'll fetch fresh from
the registry and clobber the local copy you just staged:

```bash
DSH_RUNTIME_SOURCE=<node_modules root> npm run build
```

Output: `src-tauri/target/release/bundle/nsis/DeepSeek Harness_<version>_x64-setup.exe`

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

**For a bundled-runtime install (the default, `resources/runtime` packaged into the installer)**
these travel together automatically: `tauri build`'s own `beforeBuildCommand` guarantees every
package is preceded by a fresh `resources/runtime` install pinned to `DSH_VERSION_DEFAULT` (see
"Building the installer" above), and the NSIS installer bundles that payload wholesale — so a
shell auto-update reinstalls the runtime pinned at build time along with it. There's no separate
runtime-update mechanism to build as long as `DSH_VERSION_DEFAULT` is bumped (and
`check:dsh-version` passes) before cutting each shell release.

**For the managed (non-bundled) runtime path** — used when there's no `resources/runtime/`
(e.g. an unpackaged dev build, or `DSH_DESKTOP_RUNTIME_DIR` pointed elsewhere) — the runtime is
installed once via `npm install` on first use ([server.rs](../src-tauri/src/server.rs)'s
`install_runtime`) and **never re-checked afterward**. A user on this path who wants a newer
`dsh` has to clear `DSH_DESKTOP_RUNTIME_DIR` (or set `DSH_DESKTOP_DSH_VERSION` to a newer spec)
and let it reinstall. This is a known, narrow gap — not worth a bespoke updater for a path that's
mainly used in development.

### Why the pinned version can lag behind the newest RC on npm

`check:dsh-version` compares against npm's **`latest` dist-tag**, not "the newest published
version string" — upstream publishes new RCs to `next` first (or under no tag at all), watches
them for a while before deciding whether to point `latest` at them, and sometimes abandons one
partway through, superseded by a newer version while `latest` never moves. This happened for
real: `0.1.0-rc.8` sat on `next` for two days without ever being promoted to `latest`, then got
bumped off `next` entirely by `0.1.1-rc.1` — `latest` stayed on `0.1.0-rc.7` the whole time (see
[3a55628](../commit/3a55628) for the details). Chasing "newest published version" instead of the
`latest` tag would have pinned this app to a release upstream itself never endorsed and, in
hindsight, effectively skipped.

So `DSH_VERSION_DEFAULT` only gets bumped when the `latest` tag actually moves — a new version
showing up on `next`, no matter how many days it sits there looking stable, isn't the signal to
act on. `check:dsh-version` gates on exactly that (`dist-tags`, not `versions.at(-1)`). To try an
unpromoted version yourself, override it with `DSH_DESKTOP_DSH_VERSION` (see "Environment
overrides" above) rather than changing the hardcoded default.

### The other reason to lag: 0.1.5+ can't be hosted in an iframe at all

The flip side of the rule above has an exception too, encoded in `NOT_ADOPTABLE` in
`check-dsh-version.mjs`: **a version that *is* npm's `latest` still doesn't get adopted if it
has been verified not to run in this shell**, and the script then treats being behind as the
expected outcome rather than a failure. Today that table holds exactly one entry,
`0.1.5-rc.1`.

The cause isn't a bug — it's upstream's deliberate browser authentication, and it **requires
same-site**. Measured against the `0.1.5-rc.1` build; `0.1.5-rc.2` shows no change here, but has **not** been re-verified line by line (upstream republishes these version strings, so measure against what npm currently ships before concluding anything):

- the startup line becomes `dsh web: http://127.0.0.1:<port>/?token=<secret>` — the **token is
  required**;
- `GET /` without a cookie always answers **401** `dsh web authentication required; reopen the URL
  printed by dsh web.`;
- `GET /?token=<secret>` answers **303** back to `/` and sets a `dsh-auth-<authority>` cookie
  (`HttpOnly; SameSite=Strict; Path=/`, bound to `host:port`);
- `/api/*` without the cookie is **401**; with `Sec-Fetch-Site: cross-site` it is **403**
  (`api-request-trust`).

A `SameSite=Strict` cookie is only stored and sent when the harness **is itself the top-level
document**. This repo used to host it in a **cross-site** `<iframe>` (outer page `tauri.localhost`,
iframe pointing at `127.0.0.1:<port>`), where that cookie can never take effect — leaving only that
401 line in the window. Relatedly, `server.rs` used to drop everything after the port when parsing
the startup URL; that also discarded the token, so preserving the whole URL is **required**, not
merely nice.

⚠️ **A genuinely misleading upstream behaviour**: these 0.x prerelease packages get **republished**
(same version string, different contents — `prepare-runtime.mjs`'s comment says as much). Reading
the older Sep-10 copy in `~/.dsh` (whose build had the auth unwired: `/` served unauthenticated and
the ready line had no token) once led to the false conclusion that 0.1.5 had no authentication at
all. **Judge auth behaviour against what npm currently publishes**, never against a stale local
copy.

**Adopting 0.1.5+ required an architecture change first — and that change has now landed**: the
harness renders as a **top-level document** in a child webview of the same window (Tauri's
`unstable` `Window::add_child`; see `HARNESS_WEBVIEW_LABEL` in `lib.rs`), while the shell
(toolbar/dock/overlays) keeps running in the `tauri.localhost` webview so it retains IPC. The
harness is then same-origin with itself, its `/api` calls are same-origin, and they pass that 403.
The harness still gets no Tauri IPC, since `dangerousRemoteDomainIpcAccess` stays off and remote
origins are never injected with it. Do **not** try to make the iframe same-origin by moving the
shell page onto `127.0.0.1` as well — that needs remote-IPC access granted to that origin, which
would hand it to the harness's port too and break the "harness pages get zero IPC"
boundary outright. `DSH_VERSION_DEFAULT` now tracks npm’s `latest`, and the `NOT_ADOPTABLE` table is
empty — the blocker was fixed rather than tolerated.
</content>
