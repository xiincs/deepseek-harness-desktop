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
| `DSH_DESKTOP_DSH_VERSION` | 托管运行时使用的 npm 版本号（默认 `0.1.0-rc.7`） |
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
npm run bundle        # fetch:node → prepare:runtime → tauri build
# 或者分步执行：
npm run fetch:node    # 下载 node.exe → src-tauri/resources/runtime
DSH_RUNTIME_SOURCE=<node_modules 根目录> npm run prepare:runtime  # 用本地运行时代替 npm install
npm run build         # → src-tauri/target/release/bundle/nsis/DeepSeek Harness_<version>_x64-setup.exe
```

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

**对于内置运行时的安装方式（默认，即 `npm run bundle` 打出来的包）**，这两者会自动同步：NSIS 安装包
的内容里包含 `resources/runtime/`，所以外壳的自动更新会连带把构建时打进去的那个运行时版本一起重装
——只要在每次切外壳版本发布前把 `DSH_VERSION_DEFAULT` 更新好（并且 `check:dsh-version` 检查通过），
就不需要另外再搭一套运行时更新机制。

**对于托管（非内置）运行时的路径**——也就是没有 `resources/runtime/` 的场景（比如未打包的开发版，
或者 `DSH_DESKTOP_RUNTIME_DIR` 指向了别处）——运行时只会在首次使用时通过 `npm install` 装一次
（见 [server.rs](../src-tauri/src/server.rs) 里的 `install_runtime`），**之后不会再检查**。走这条路径
的用户如果想用更新的 `dsh`，得自己清空 `DSH_DESKTOP_RUNTIME_DIR`（或者把 `DSH_DESKTOP_DSH_VERSION`
设成更新的版本号）让它重装。这是一个已知但影响面很窄的缺口——这条路径主要在开发时用得到，不值得为它
单独搭一套更新机制。
</content>
