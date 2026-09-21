# dsh-desktop

Tauri（Rust + WebView2）桌面壳，托管 `@deepseek-ai/dsh` 的 `dsh web` 服务并把窗口指向它。真正的界面是 `dsh web` 提供的网页（本机 `http://127.0.0.1:<port>`），这个仓库只是启动/管理它的原生外壳，外加右侧一个原生（非 iframe 内）的**文件预览/终端/Diff/插件市场** dock。

面向终端用户的产品介绍见 [README.md](README.md)；搭建开发环境、打包发布见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。这份文档只写给未来在这个仓库里工作的开发者/AI 助手：架构、代码里不写在注释里就容易被忽略的隐性约束、以及会反复浪费时间的坑。不记录"最近一轮做了什么"这类会过时的内容——那类信息看 `git log`，或找 `docs/internal/HANDOFF.md`（每轮开发结束会被重写，只反映最新一轮）。

## 架构

```
┌─ Tauri 窗口 (Rust, src-tauri/) ─────────────────────────────────┐
│ ┌─ shell webview（tauri.localhost，有 IPC）───────────────────┐ │
│ │ ui/index.html + app.js：工具栏 / 加载中 / 出错 / 重试 / dock │ │
│ │   （文件预览、终端、Diff、插件市场、各覆盖层）+ invoke() 调用 │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─ harness webview（同窗口子 webview，顶层文档，零 IPC）──────┐ │
│ │ 就绪后导航到 → http://127.0.0.1:<port>（dsh web 的界面）     │ │
│ │   ← 工作区文件树现在由它自己画（右侧 Sidebar），不是外壳      │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ 服务管理器 (server.rs)：定位 node → 装/校验 dsh 运行时 → 探测端口  │
│   → 拉起进程 → 解析 stdout 里的真实 URL → 监视 → 退出时杀进程树    │
│ 原生菜单/托盘 (menu.rs)、dock 的 git/文件读写 (panel.rs)          │
└─────────────────────────┬───────────────────────────────────────┘
                          │ 拉起子进程
                 ┌────────▼────────┐
                 │  dsh web 服务    │  数据 → ~/.dsh (DSH_HOME)
                 └─────────────────┘
```

`ui/` 只有一层：`index.html` + `app.js` + `styles.css`，无构建步骤、无框架，纯手写 DOM。

窗口里是**两个 webview**（`lib.rs` 的 `HARNESS_WEBVIEW_LABEL`）：`ui/` 那套跑在 `shell` webview（`tauri.localhost`，有 IPC），harness 跑在一个**子 webview** 里、作为**顶层文档**加载 harness URL。

**为什么不能再用 `<iframe>`**：dsh 0.1.5 起，harness 整个界面挡在浏览器认证后面，而认证要求同站（实测 npm 当前发布的 `0.1.5-rc.1`）：
- 启动行改成 `dsh web: http://127.0.0.1:<port>/?token=<secret>`，**token 必填**——所以 `server.rs` 的 `extract_ready_url` 原样保留整条 URL（只取端口重建就会丢掉 token，直接 401）。
- 不带 cookie 的 `GET /` 一律 **401** `dsh web authentication required; …`；`GET /?token=<secret>` 会 **303** 到 `/` 并种下 `dsh-auth-<authority>` cookie（`HttpOnly; SameSite=Strict; Path=/`，绑定 `host:port`）。
- `/api/*` 不带 cookie 是 **401**，带 `Sec-Fetch-Site: cross-site` 是 **403**（`api-request-trust`）。

`SameSite=Strict` 的 cookie 只有在 harness **自己就是顶层文档**时才存得下、带得上；挂在 `tauri.localhost` 下的 iframe 是跨站，cookie 永远不生效，窗口里只会剩那行 401 文案。改成顶层文档后 harness 与自身同源，cookie 正常，认证才通过、`/api` 才走得通。

harness 那个 webview **拿不到 Tauri IPC**（`dangerousRemoteDomainIpcAccess` 从不开启，且它加载的是外部来源，按构造就没有）——所有外壳层面的操作（文件预览/读写、终端、Diff、插件市场、菜单/托盘）都是 `ui/app.js` 在 shell webview 里直接调 `invoke()`，harness 页面本身接触不到桌面能力。

代价是 harness 是**兄弟原生 webview，盖在 shell 页面之上**，所以：① 插件市场/对话框/汉堡菜单这些 DOM 覆盖层会被它挡住，必须靠 `app.js` 的 `syncHarnessVisibility()` 在覆盖层打开时把 harness 藏起来（`#harness-scrim` 负责不露白屏）；② 外壳那些 WebView2 控制级设置（禁右键菜单/锁缩放/剪贴板授权/外链路由）**不再自动覆盖 harness**，必须在建 harness webview 时单独设一遍（见 `lib.rs` 建子 webview 那段）；③ 几何靠 `app.js` 用 `ResizeObserver` 上报 `harness_set_bounds` 来同步。

## 关键约束（会反复踩，务必记住）

- **改了 `ui/*` 任何文件，必须 `cargo build`（或 `npm run tauri dev`/`build`）才会生效，不能只重跑已有的 `.exe`。** `tauri.conf.json` 的 `frontendDist` 指向 `../ui`，Tauri 在**编译期**把整个 `ui/` 目录内嵌进二进制（`tauri.localhost` 自定义协议从内嵌资源读，不是运行时读磁盘）。改完前端代码却"重跑旧进程"是这个项目里最容易踩、最容易误诊为"代码有 bug"的坑——先确认是不是在跑旧二进制，再往 CSS/JS 逻辑里找。
- **Windows 上调用 npm 生态的 CLI（npm/pnpm/…）不能直接 `Command::new("npm")`。** 这些工具在 Windows 上装成 `.cmd`/`.ps1` shim，`Command::new` 直接调 `CreateProcess`，不会像交互式 shell 那样查 `PATHEXT` 去尝试 `.cmd` 后缀，会报"program not found"。项目里正确的调法是 `server.rs` 的 `npm_ecosystem_command(program, args)`（Windows 上转发到 `cmd /C <program> <args>`），新增任何 shell 出去调这类工具的代码都要走这个，不要重新手写 `Command::new`。
- **`effective_path()`（`server.rs`）会展开注册表 PATH 里的 `%VAR%` 占位符，但这需要显式调用 `expand_env_string`（Win32 `ExpandEnvironmentStringsW`）。** `winreg` 的 `get_value::<String, _>` 对 `REG_EXPAND_SZ` 类型的值只做原样字符串解码，从不展开——如果直接读注册表 PATH 而不经过这层展开，任何装在 `%SOME_VAR%\...` 这类路径下的工具（常见于 nvm-for-windows 等版本管理器）会在子进程里彻底找不到，且没有任何报错线索指向真正原因。这是这个项目里已经踩过一次、排查耗时最长的坑。
- **`server.rs` 里所有子进程启动，PATH 都要显式设成 `effective_path()` 的返回值，不能依赖继承的进程环境。** 从 Explorer/开始菜单启动的 GUI 进程，PATH 只在登录时计算一次；装了新工具但没注销重登，旧进程（以及它派生的所有子进程）看到的 PATH 是过时的，可能完全找不到刚装好的工具。
- **`docs/DEVELOPMENT.md` 里写的环境变量覆盖项（`DSH_DESKTOP_NODE`/`DSH_DESKTOP_DSH_BIN`/`DSH_DESKTOP_RUNTIME_DIR` 等）是诊断"找不到 node/dsh"类问题时的第一入口**，不要在 `resolve_node`/`resolve_bin` 之外另起一套定位逻辑。
- **测试原生窗口时，关闭方式必须带上子进程清理。** `taskkill` 要 `/F /T`（不带 `/T` 会留下孤儿 `dsh` Node 服务，下次重新编译/启动时可能撞端口冲突或读到脏状态）。
- **`.setup()` 里绝不能调用阻塞式的 WebView2 异步等待**（`webview2_com` 的 `*CompletedHandler::wait_for_async_operation`）。`setup` 跑在事件循环线程上，这种等待会一直阻塞到那个 WebView2 异步操作完成——而完成它需要事件循环继续转，于是互等死锁。**症状极具误导性**：窗口起来了但内容空白、stdout **一行都没有**（连 `[dsh-desktop] spawned dsh pid …` 都不打印，因为 `setup` 末尾的 `thread::spawn(server::start)` 根本没执行到）、端口 3080 不 LISTENING——看着特别像上面第一条"改了 `ui/` 却跑了旧二进制"，其实是死锁。注册类调用不需要返回值就**即发即忘**（建个 handler 直接调 `Add*`，别 `wait_for_async_operation`）。判断手法：stdout 一行都没有 ≈ 卡在 `setup`；有日志 ≈ 已经过了 `setup`，往别处找。
- **窗口级操作（show/hide/focus/unminimize/decorations）必须用 `app.get_window("main")`——`lib.rs` 里统一走 `main_window()`，不要用 `app.get_webview_window("main")`。** Tauri 2 的 `get_webview_window` 在按 label 查表之外还要过一道 `Window::is_webview_window()`：只有"该窗口里**每个** webview 的 label 都等于窗口 label"时才返回 `Some`。harness 独立成同窗口的第二个子 webview（label `harness`）之后，窗口 "main" 里有两个 webview，这个查找**恒为 `None`**，于是所有 `if let Some(win) = app.get_webview_window("main")` 一起退化成静默空操作——关闭对话框的"最小化到托盘"点了没反应（`resolve_close_choice`）、托盘/热键/二次启动都唤不回窗口（`show_main_window`）、`setup` 里给 shell webview 装的那批 WebView2 控制级设置（禁右键/锁缩放/禁 DevTools/剪贴板授权/外链路由）连同 `window_proc::patch_maximized_hit_test` 整块被跳过。webview 级操作用 `app.get_webview(label)`（`harness_*` 那几个一直是对的）。
- **工作区文件树的唯一实现在 harness 里（`@deepseek-ai/dsh-client-ui-sidebar-files`，右侧 Sidebar），外壳不再画第二棵树，也不许再加回来。** 曾经两棵树各按自己的根渲染（外壳那棵走 `panel::effective_workspace_dir` 的推断，内核那棵走会话的 `cwd`），同一份工作区在两边显示不同内容——这正是被反复报的那个问题。删掉外壳那棵之后连带消失的东西：`get_workspace_tree`/`get_active_workspace` 两个命令、dock 的工作区选择器、树右键菜单、拖拽移动、以及 `panel::list_workspace_tree` 一整块。内核那棵树**只读**（README 里明说：无搜索、无重命名、无右键菜单、无编辑），所以外壳保留了 `#card-file` 预览卡（CodeMirror 编辑器，可改可存）——这是它唯一还没被内核覆盖的能力，别顺手也删了。
- **dock 的「显示/隐藏」由 `ui/app.js` 的 `syncPanelVisibility()` 决定，不是 `setDockView(view)` 里那句 `view === null`。** `#card-file`（文件预览）不是可切换的 view 之一：它常驻 `#panel-cards`，和终端/Diff 并排，也可能**是唯一开着的东西**（终端最后一个 tab 关掉时会调 `setDockView(null)`）。按 `view === null` 隐藏整个 dock 会把用户正在看的文件一起藏掉；反过来只关预览而不重新同步，又会把 dock 留成一条空栏。新增/删除任何卡片后都要回来看这个函数。
- **手动验证原生窗口界面的改动（截图、点击、拖拽）应该让用户直接在真实窗口里操作确认，不要依赖自动化工具反复截图/读无障碍树来回合调试。** 桌面自动化对这个项目某些原生窗口的截图本身不可靠（历史上记录过纯黑图），反复来回的诊断成本远高于直接问用户"你看到的是什么"。
- **原生 shell 的 zh/en 双语（`i18n.rs` + `ui/app.js` 里的 `STRINGS`/`t()`）跟 `dsh web`（harness webview 里那个网页）自己的 `locale.preference` 设置是两套完全独立、互不感知的机制，不是同一个开关。** 两边各自从系统/浏览器语言探测一次（Rust 侧 `sys_locale::get_locale()`，JS 侧 `navigator.language`），规则一致（探测到 `en*` 才用英文，其余一律回退中文），但没有任何代码把二者同步在一起——harness webview 里那个网页是完全隔离的远程页面，永远没有 Tauri IPC（见 `lib.rs` 顶部模块注释），这层壳读不到它的 `locale.preference`，也不该为此专门开一个通信通道。想要"跟随应用内语言设置"是行不通的；如果这个假设第二次被提出，说明这条边界需要重新在这里写清楚，而不是去改代码硬凑。

## 插件市场（`ui/app.js` 里 `plugin-market-*`，`server.rs` 里 `install_plugin`/`check_pnpm_available`/`install_pnpm`）

从 `awesome-dsh-plugin.com/plugins.json` 动态抓取的一份**众包、未经审核**的插件列表，不是这个仓库自己维护的白名单。安装机制是薄封装 `dsh plugin --profile web add <package>`（转发到 `pnpm add`），真正执行第三方代码、有真实供应链风险（安装脚本、任意 npm 包）。任何改动这部分的人都要保留：

- 安装前必须经过确认视图（展示确切要跑的命令 + 风险提示），不能做成点卡片就直接装的"一键安装"。
- 插件的 `install` 字段（[server.rs](src-tauri/src/server.rs) 的 `install_plugin` doc comment 有更完整的记录）只允许来自这份已确认过的目录，`open_external_url` 命令也硬编码限制只能打开 `https://github.com/` 开头的链接——这些都是刻意的窄化，不要为了"更灵活"放宽。
- 装插件依赖 `pnpm` 在 PATH 里；`check_pnpm_available`/`install_pnpm` 是检测+一键修复这个前置条件的配套逻辑，跟上面"Windows 调 npm 生态 CLI"的约束共用同一套 `npm_ecosystem_command`。

## 目录速查

- `src-tauri/src/server.rs` — dsh 子进程生命周期、PATH 处理、插件安装、pnpm 检测
- `src-tauri/src/panel.rs` — dock 的 Git 状态、文件读写/预览、工作区根目录解析（文件树本身见上面"关键约束"）
- `src-tauri/src/menu.rs` — 原生菜单栏 + 托盘
- `src-tauri/src/terminal.rs` — 内嵌终端（ConPTY/winpty）
- `src-tauri/src/i18n.rs` — 原生 shell（菜单/托盘/通知/少量错误提示）的 zh/en 探测与翻译，`ui/app.js` 里的 `STRINGS`/`t()` 是同一套规则在前端的独立实现
- `ui/app.js` — 唯一的前端逻辑文件，启动页状态机、dock 面板、CodeMirror 编辑器、终端 xterm.js、插件市场全在这一个文件里
- `docs/internal/HANDOFF.md` — 最近一轮开发的交接记录，每轮重写，读之前先看 git log 确认没有更新的版本
