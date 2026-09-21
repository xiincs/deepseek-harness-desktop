# 开发指南

中文 | [English](DEVELOPMENT.en.md)

面向想要参与开发、自行构建，或者只是想搞清楚这个应用内部是怎么运作的读者。日常使用请看
[主 README](../README.md)。

## 开发环境要求

- [Rust](https://rustup.rs/)（MSVC 工具链）——用于构建 Tauri 外壳
- [Node.js](https://nodejs.org/) >= 22——`dsh` 本身依赖（应用会从 `PATH` 里定位它）
- [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 已预装，
  多数 Windows 10 也已预装）

## 开发

```bash
npm install          # 安装 @tauri-apps/cli
npm run tauri dev    # 编译 Rust 外壳并打开应用窗口
```

首次启动时，应用会把 `@deepseek-ai/dsh` 这个 npm 包安装到每用户独立的运行时目录
（`%LOCALAPPDATA%\dev.dsh.desktop\runtime`）并启动它。安装结果会被 npm 缓存，所以第二次启动会很快，
且不需要联网。

### 环境变量覆盖项

| 变量 | 作用 |
|---|---|
| `DSH_DESKTOP_NODE` | 指定 `node.exe` 的绝对路径，代替 `PATH` 上那个 |
| `DSH_DESKTOP_DSH_BIN` | 指定某个 `dsh` `lib/bin.js` 的绝对路径（比如本地某个 checkout） |
| `DSH_DESKTOP_RUNTIME_DIR` | 托管的 `@deepseek-ai/dsh` 运行时安装位置（默认是应用缓存目录）；指向一个已有的 `node_modules` 根目录可以跳过首次的 npm install |
| `DSH_DESKTOP_DSH_VERSION` | 托管运行时使用的 npm 版本号（默认 `0.1.5-rc.2`，与 npm 的 `latest` 保持一致；想尝鲜未转正的版本时用它覆盖） |
| `DSH_DESKTOP_PORT` | 默认绑定端口覆盖（默认 `3080`）；同时跑多个实例时很有用 |
| `DSH_DESKTOP_CWD` | `dsh` 服务进程的工作目录（默认是用户主目录） |
| `DSH_HOME` | 透传给服务端；harness 数据根目录（默认 `~/.dsh`） |

## 架构

```
┌─ Tauri 应用 (Rust, WebView2) ─────────────────────────────┐
│ 本地启动页（加载中 / 出错 / 重试）                          │
│   └─ 就绪后跳转到 → http://127.0.0.1:<port>（真正的界面）  │
│ 服务管理器 (src-tauri/src/server.rs)                       │
│   定位 node → 安装/校验 dsh 运行时 → 探测 3080 端口         │
│   → 拉起 `node dsh web --port …` → 从 stdout 解析真实 URL   │
│   → 跳转 → 监视进程 → 退出时 taskkill 整棵进程树             │
│ 原生菜单与托盘 (src-tauri/src/menu.rs)                      │
└─────────────────────────┬────────────────────────────────┘
                          │ 拉起
                 ┌────────▼────────┐
                 │  dsh web 服务    │  数据 → ~/.dsh (DSH_HOME)
                 └─────────────────┘
```

harness 页面从 `http://127.0.0.1:<port>` 加载，故意**不**授予 Tauri IPC 访问权限
（`dangerousRemoteDomainIpcAccess` 始终不开启），所以 Web 界面本身接触不到桌面外壳——所有外壳层面的
操作都得走原生菜单/托盘，或者本地启动页。

## 打包安装程序

```bash
npm install
npm run build          # tauri build 前会自动跑 fetch:node + prepare:runtime，见下方原因
# npm run bundle 是同一件事的显式别名，效果一样
```

`tauri.conf.json` 的 `beforeBuildCommand` 把 `fetch:node`/`prepare:runtime` 接到了 `tauri build`
本身之前，不管是 `npm run build`、`npm run tauri build` 还是 `cargo tauri build` 都逃不掉。这是刻意的：
`src-tauri/resources/runtime` 是 gitignored、手动生成的产物，加这道 hook 之前，绕开 `npm run bundle`
直接跑 `tauri build` 会静默打包磁盘上现有的版本，哪怕它跟 `DSH_VERSION_DEFAULT` 早就不一致——本地
已装的应用就是这么崩的：`resources/runtime` 停在了旧版本，从未被强制跟代码同步过，用户直接撞见运行时
根本不认识的命令行参数。

用本地 checkout 代替 npm registry 时（`DSH_RUNTIME_SOURCE`），这个变量在打包那一步也要带上，不能只加
在手动跑的 `prepare:runtime` 上——`beforeBuildCommand` 打包前会再跑一次 `prepare:runtime`，没看到这
个变量就会直接从 npm registry 重装，把刚暂存的本地版本覆盖掉：

```bash
DSH_RUNTIME_SOURCE=<node_modules 根目录> npm run build
```

打包产物：`src-tauri/target/release/bundle/nsis/DeepSeek Harness_<version>_x64-setup.exe`

正式发布前先跑一下 `npm run check:dsh-version`——上游还在开发者预览阶段，会毫无预警地发布新的 RC；
这个脚本会检查写死的 `@deepseek-ai/dsh` 默认版本号（在 `src-tauri/src/server.rs` 和
`scripts/prepare-runtime.mjs` 里各存了一份，两边必须一致）是否落后于 npm 上的最新版本。发布 CI
workflow 也会跑同一个检查，版本号对不上会直接让构建失败。

### 两条独立的版本轴线

这个应用有两个互相独立、不能混为一谈的版本号：

- **外壳版本**（`tauri.conf.json` 里的 `version`）——桌面外壳本身的版本。
  `tauri-plugin-updater` 只会更新这一个。
- **运行时版本**（`server.rs` 里的 `DSH_VERSION_DEFAULT` / `prepare-runtime.mjs` 里的默认值）——
  打进安装包或首次运行时安装的那个 `@deepseek-ai/dsh` 版本。

**对于内置运行时的安装方式（默认，即 `resources/runtime` 打进安装包）**，这两者会自动同步：`tauri
build` 自身的 `beforeBuildCommand` 保证每次打包前都会用 `DSH_VERSION_DEFAULT` 重新装一遍
`resources/runtime`（见上面"打包安装程序"），NSIS 安装包再把它整个打进去，所以外壳的自动更新会连带
把构建时那个运行时版本一起重装——只要在每次切外壳版本发布前把 `DSH_VERSION_DEFAULT` 更新好（并且
`check:dsh-version` 检查通过），就不需要另外再搭一套运行时更新机制。

**对于托管（非内置）运行时的路径**——也就是没有 `resources/runtime/` 的场景（比如未打包的开发版，
或者 `DSH_DESKTOP_RUNTIME_DIR` 指向了别处）——运行时只会在首次使用时通过 `npm install` 装一次
（见 [server.rs](../src-tauri/src/server.rs) 里的 `install_runtime`），**之后不会再检查**。走这条路径
的用户如果想用更新的 `dsh`，得自己清空 `DSH_DESKTOP_RUNTIME_DIR`（或者把 `DSH_DESKTOP_DSH_VERSION`
设成更新的版本号）让它重装。这是一个已知但影响面很窄的缺口——这条路径主要在开发时用得到，不值得为它
单独搭一套更新机制。

### 为什么锁定的版本可能落后于 npm 上"最新发布"的 RC

`check:dsh-version` 比对的是 npm 的 **`latest` dist-tag**，不是"最新发布的版本号"——上游会先把新
RC 发到 `next`（或者不挂任何 tag），观察一段时间再决定要不要把 `latest` 指过去，也可能中途放弃，
被更新的版本直接顶替、`latest` 全程不动。真实发生过一次：`0.1.0-rc.8` 发布后在 `next` 上挂了两天，
从未被提升为 `latest`，随后又被 `0.1.1-rc.1` 顶替掉了 `next` 的位置，`latest` 全程停在
`0.1.0-rc.7`（细节见 [3a55628](../commit/3a55628)）。如果当初追的是"最新发布版本号"而不是
`latest` 标签，这个仓库会一路追着一个上游自己都没有背书、事后看等于被跳过的版本。

所以 `DSH_VERSION_DEFAULT` 只在 `latest` 标签真正移动时才跟进——`next` 上出现新版本，哪怕挂了
好几天看起来很稳定，都不是该跟进的信号。`check:dsh-version` 脚本就是照这个信号做门控的（查
`dist-tags`，不是 `versions.at(-1)`）。想手动尝鲜一个还没转正的版本，用
`DSH_DESKTOP_DSH_VERSION` 覆盖（见上面"环境变量覆盖项"），不要改动写死的默认值。

### 0.1.5 起 harness 必须当顶层文档承载（为什么本仓库为此改过架构）

反面的例外也留着一个位置，写在 `check-dsh-version.mjs` 的 `NOT_ADOPTABLE` 表里：**即使某个版本已经
是 npm 的 `latest`，只要验证过这个外壳跑不起来，就不跟进**，脚本把落后当成预期结果放行。它曾经用来
豁免 `0.1.5-rc.1`，**现在表是空的**——因为障碍是被修掉的，不是被容忍的（下详）。

原因不是 bug，是上游刻意的浏览器认证，而且它**要求同站**。下面这几条是对 `0.1.5-rc.1` 实测的；`0.1.5-rc.2` 未见改动，但**没有重新逐条验证过**（上游会重新发布同一版本号的内容，真要下结论以当前 npm 发布的那份为准）：

- 启动行变成 `dsh web: http://127.0.0.1:<port>/?token=<secret>`，**token 是必填的**；
- 不带 cookie 的 `GET /` 一律返回 **401** `dsh web authentication required; reopen the URL printed
  by dsh web.`；
- `GET /?token=<secret>` 返回 **303** 跳回 `/`，并种下 `dsh-auth-<authority>` cookie
  （`HttpOnly; SameSite=Strict; Path=/`，绑定 `host:port`）；
- `/api/*` 不带 cookie 是 **401**，带 `Sec-Fetch-Site: cross-site` 是 **403**（`api-request-trust`）。

`SameSite=Strict` 的 cookie 只有在 harness **自己就是顶层文档**时才存得下、带得上。本仓库原本
把 harness 放在**跨站** `<iframe>` 里（外层 `tauri.localhost`，iframe 指向 `127.0.0.1:<port>`），
cookie 永远不生效，于是窗口里只剩那行 401 文案。顺带一提，`server.rs` 解析启动 URL 时曾经把
端口之后的内容丢掉（只取端口重建 URL）——那样连 token 都没了，**必须**原样保留整条 URL。

⚠️ **一个会坑人的上游行为**：npm 上这些 0.x 预发布包会被**重新发布**（同一个版本号、不同内容），
`prepare-runtime.mjs` 的注释里也写了这一点。曾经因为读的是 `~/.dsh` 里 9 月 10 日的旧副本
（那一版的认证没有接线，`/` 免认证、ready 行无 token），误判成"0.1.5 根本没有认证"。
**判断认证行为时一定要以当前 npm 发布的那份为准**，别拿本地旧副本下结论。

**所以 0.1.5+ 的前提是改架构，而这项改造已经落地**：harness 现在作为**顶层文档**渲染在同一个窗口的
子 webview 里（Tauri `unstable` 功能的 `Window::add_child`，见 `lib.rs` 的 `HARNESS_WEBVIEW_LABEL`），
外壳（工具栏/dock/覆盖层）继续跑在 `tauri.localhost` 的 webview 里以保留 IPC。这样 harness 是顶层
文档，`SameSite=Strict` 的 cookie 才存得下、带得上，认证与 `/api` 才走得通；同时 harness 依旧拿不到
Tauri IPC（`dangerousRemoteDomainIpcAccess` 保持关闭，远程来源不注入 IPC）。注意**不要**为了让 iframe
同源而把外壳页面也搬到 `127.0.0.1` 上——那需要给这个来源开远程 IPC 权限，会连带把 harness 的端口也
放进来，直接破坏"harness 页面零 IPC"这条边界。`DSH_VERSION_DEFAULT` 现已跟到 npm 的 `latest`。

**两个随架构而来的副作用**（写在 CLAUDE.md 里，改动时别丢）：harness 子 webview 是盖在 shell 页面
之上的兄弟原生 webview，会挡住 DOM 覆盖层（靠 `app.js` 的 `syncHarnessVisibility()` 遮挡时隐藏）；
外壳那些 WebView2 控制级设置也不再自动覆盖 harness，建子 webview 时要单独设一遍。
</content>
