# Agent Note：用于内核管理的 Tauri 桌面外壳

Status: implemented

[English](2026-08-21-tauri-desktop-shell.md) | 中文

## 问题

DeepSeek Harness 目前的消费方式是通过浏览器 UI 使用：`npx @deepseek-ai/dsh web`（或 `pnpm dsh web`）在 `http://127.0.0.1:3080` 启动。用户需要一个桌面外壳：(1) 在桌面上自由打开 harness；(2) 提供一个内核更新菜单，跟随官方 `deepseek-ai/deepseek-harness` 的 Release tag 原位安装与切换内核版本；(3) 通过 GitHub Actions 面向 Intel macOS 与 Windows 发布。官方 GitHub Release 只附带源码包；真正的内核以 `@deepseek-ai/dsh` npm 包（连同数十个 `@deepseek-ai/dsh-*` 依赖包）分发，版本与 `dsh-v<semver>` tag 一一对应。因此桌面端无法从 release asset 直接拿到"内核"——必须安装 pinned 到该 tag 版本的 npm 包。

## 决定

**外壳是独立于 pnpm workspace 的 `desktop/` 目录树。**它从不进入 `pnpm-workspace.yaml`，于是上游的锁文件、lint/hygiene/release 序列和打包门禁都不会触及它，也不会被它破坏不变量。它是独立的交付物，带自己的 `package.json`（仅 `@tauri-apps/cli`）和自己的 Rust crate。

**Tauri v2 承载三个窗口。**主窗口加载零构建的静态 `ui/` 资源（本地管理面板：状态、更新菜单、设置、日志，以及「打开官方对话」按钮）。两个按需 `WebviewWindow` 由用户在面板里点击按钮拉起：`harness` 加载 `http://127.0.0.1:<port>`——即 `dsh web` 服务——让工作台在独立窗口运行；`official-chat` 加载 `https://chat.deepseek.com`，让 DeepSeek 官方对话在桌面上打开而不用离开外壳。重复点击任一按钮都会通过 `get_webview_window` + `set_focus` 复用既有窗口。没有前端构建步骤；`frontendDist` 指向 `ui/`，`withGlobalTauri` 为面板的纯 JS 暴露 `window.__TAURI__.core` 桥。

**官方对话窗口以诚实的桌面版 Edge 身份运行，让 chat.deepseek.com 的环境检查把它当普通桌面浏览器。**`open_official_chat` 不再覆盖 user-agent：WebView2 引擎本身就是真实的桌面版 Edge，改写 UA 去声称 Chrome 会与不可覆盖的 `Sec-CH-UA` 客户端提示及原生 `navigator.userAgentData` 相矛盾——这种跨层不一致正是环境检测的特征。所有平台都调用 `.additional_browser_args(OFFICIAL_CHAT_BROWSER_ARGS)`（仅 WebView2 后端消费该参数）：重述 wry 默认禁用项（`msWebOOUI`、`msPdfOOUI`、`msSmartScreenProtection`——自定义参数会整体替换默认值）并叠加 `AutomationControlled`、`TranslateUI`、`InterestFeedContentSuggestions` 与 `--disable-blink-features=AutomationControlled`。自定义参数要求专属 user-data 目录——WebView2 要求同一目录上的环境选项完全一致，而面板/工作台已在默认目录用默认选项建好环境——所以 builder 把 `.data_directory` 固定到 `<data_dir>/webview-official-chat`。专属目录同时充当持久化配置档案：DeepSeek 登录态跨外壳重启保留，且与面板/工作台窗口相互隔离。Tauri 2.11 builder 顺序仍是关键：属性调用必须早于每一个 `initialization_script` 且早于 `.build()`；三个初始化脚本按添加顺序运行——`pullstring-launcher.js`（先把真实 `window.__TAURI__` 捕获到闭包变量 `__DSH_TAURI_REF__`）→ `titlebar-pulse.js`（挂 chrome-row 顶部条带）→ `chat-fingerprint.js`（最后跑，把 `navigator.webdriver` 钉在 `false`，并彻底删除 `__TAURI__` / `__TAURI_INTERNALS__` / `__TAURI_METADATA__` / `__TAURI_IPC__`——正常浏览器里这些全局根本不存在，哪怕是拒绝调用的 neutered Proxy 也仍在宣告嵌入式身份）。拉绳挂件凭闭包里捕获的引用在全局删除后依然可用。命令改为 `async`，builder 结果通过 `std::sync::mpsc::channel` 回到 `tauri::async_runtime::spawn_blocking` 等待，IPC 调用者只在窗口真正注册后才 `Ok(())`。面板读取 `StatusView.official_chat_open`（2.5s 轮询），把同一按钮文案在「打开官方对话」与「关闭官方对话」之间切换；窗口已开时再次点击触发新增的 `close_official_chat` 命令（销毁已注册的 webview，或在未注册时返回「官方对话窗口未打开」错误）。

**内核版本是钉在应用数据目录下的 npm 安装。**`fetch_releases` 通过 GitHub REST API 读取官方发布列表，API 被限流时回退到 releases Atom feed（tag 相同；回退仅丢失 prerelease 标记）。安装某版本执行 `npm install --prefix <app_data>/kernels/<version> @deepseek-ai/dsh@<version>`；活动版本是一个纯文本文件 `<app_data>/active.txt`。首次安装自动激活并自动启动；后续安装不动当前活动版本。因此"更新菜单替换内核"的方式是钉住另一版本并切换活动指针，而不是修补既有安装。

**Node 来自环境、可配置。**外壳从设置、再到 PATH、再到常见安装位置解析 `node`，并按 dsh 的 engine 范围（`^22.19 || >=24`）校验；npm 依次从设置、`node` 同目录、PATH 解析。这让 POC 免于捆绑 Node sidecar。

**发布是专用的 workflow。**`.github/workflows/desktop-release.yml` 用 `tauri-apps/tauri-action` 在 `macos-13`（Intel macOS，`.dmg`）与 `windows-latest`（`.exe`/NSIS）构建，由 `desktop-v*` tag 或手动触发，产出草稿 release 交给人工发布。

**工作台与官方对话窗口共享同一套 chrome-row 顶部条带 + 拉绳挂件，按 origin 切换配色。**`titlebar-pulse.js` 与 `pullstring-launcher.js` 通过 `WebviewWindowBuilder::initialization_script` 注入到 `harness` 与 `official-chat` 两个 webview。两个脚本都按 `location.hostname` 选调色板与偏移：`chat.deepseek.com` → DeepSeek 官方蓝 `#4D6BFE`（rgb 77,107,254），拉绳贴 `left:12px`；其他（即 dsh web 工作台的 `127.0.0.1`）→ Gitea 绿 `#609926`，拉绳贴 `left:212px`（侧栏折叠按钮旁）。titlebar 脚本在页面自身不带 `<body><div data-titlebar-pulse="2">` 时（chat 不带）由 `ensureSecondBar()` 补出半周期偏移节点，保证两个 sweep 周期相同（6.01s）下两条带同时出现。两个脚本顶部都有 `if (window.top !== window.self) return` 顶帧守卫，避免 Tauri 在每个 iframe 都执行初始化脚本时挂出多份拉绳 / 条带。每个远程 origin 都有专属 capability（`harness-remote.json` / `official-chat-remote.json`），只授权 `allow-focus-main-shell`；URL 锁精确主机（`http://127.0.0.1:*` 与 `https://chat.deepseek.com/*`），不开 wildcard 域名。`harness` 窗口 `closable(false)` 防止误关丢内核会话；`official-chat` 不设——它不持有内核会话，OS chrome 关闭按钮必须正常工作。

**内核子进程在退出时被回收。**Unix 上它以独立会话启动（`setsid`），停止时信号发给整个进程组；Windows 上用 `taskkill /T /F` 拆除整棵进程树。

## 曾考虑的替代方案

**把 `apps/desktop` 放进 pnpm workspace。**否决：仓库的包约束、lint 面和发布/打包序列都会扫描 workspace 成员；把 Rust/Tauri 交付物放进去会把外来工具链拖进上游门禁，其 `package.json` 还得满足 workspace 不变量。独立 `desktop/` 目录树是 fork 自有产品爆炸半径最小的归宿。

**捆绑 Node sidecar 做到开箱即用。**POC 否决：每个平台需要各自的 Node 二进制（每目标 +40 MB），外壳还得自举它，而且本地环境完全无法验证 Rust 构建。环境检测 node + 可配置路径是可逆的默认；sidecar 槽位留作后续项记录在文档。

**从 GitHub release 拉源码 tarball 当内核。**否决：桌面应用不可能在用户机器上跑 `pnpm install` + 整套 harness 构建；发布契约本来就在 npm，一个 `dsh-v*` tag 对应一个版本。

## 后果

外壳承载三个 webview：本地静态管理面板（启动时唯一存在，是管理内核的唯一入口），以及由面板上对应按钮按需拉起的两个 `WebviewWindow`——`harness` 承载 `dsh web`，`official-chat` 承载 `https://chat.deepseek.com`。`official-chat` 窗口不持有内核会话，OS chrome 关闭按钮保持可用；`harness` 窗口设 `closable(false)` 防止误关丢会话，面板靠 `get_webview_window` + `set_focus` 按需重开。内核数据（会话、设置、profile）留在 dsh 自己的 `~/.dsh` home；只有外壳元数据（已装版本、活动指针、设置、日志）位于应用数据目录。GitHub REST 回退到 Atom 只在更新菜单里以警告形式可见。安装包未签名，因此 Windows SmartScreen 与 macOS Gatekeeper 会告警，直到加入代码签名。面板每次刷新状态都通过探测端口自动恢复陈旧状态。

管理面板关闭时，`official-chat` 窗口在每次真实退出中级联关闭：它是面板驱动的瞬态窗口，不持有自己的生命周期，因此 `RunEvent::Exit` 处理路径会通过 `get_webview_window("official-chat").destroy()` 销毁它。`WindowEvent::CloseRequested` 拦截只挂起关闭动作并在内核运行或对话窗口打开时请求确认——退出前不销毁任何窗口，取消提示即完整恢复原状。`harness` 工作台窗口刻意不参与级联——它的生命周期由 `stop_kernel` 绑定到内核子进程，目前没有把它与管理面板绑死的文档化信号，所以关闭面板（暂）不关闭工作台。

## 测试

配置文件 JSON 已用 `python3 -m json.tool` 校验。应用图标基于官方 DeepSeek favicon（`desktop/assets/whale-favicon.svg`，黑色身体，取自 `https://fe-static.deepseek.com/platform/favicon.svg`），以矢量圆方式加了一只红色眼睛（成品 `desktop/assets/whale-icon.svg`）；`src-tauri/icons/` 位图集由该 SVG 的 512px 浏览器栅格化结果（`desktop/assets/whale-icon-512.png`）降采样生成，并用 `file` 校验。更新菜单的版本列表现在来自 npm registry（`https://registry.npmjs.org/@deepseek-ai/dsh`，对应用户访问的 `https://www.npmjs.com/package/@deepseek-ai/dsh` 包页面）；`fetch_npm` 解析 `versions` 与 `dist-tags`，prerelease 标记与发布时间都是权威数据。GitHub Releases (`fetch_api`) 与其公共 Atom feed (`fetch_atom`) 仅在 registry 不可达时作为 fallback；UI 通过 `warning` 字段提示当前正在用哪条数据源。当前代码显式避开了四个 Tauri 2.x 启动陷阱：`WebviewWindowBuilder` 由 `open_harness` 在新 OS 线程中创建，避免在同步命令里卡死 Windows 主线程；`macOS.infoPlist.NSAppTransportSecurity` 被省略，使用平台默认本地网络策略；`capabilities/default.json` 不硬编码尚未生成的 `$schema` 路径；`ureq` 启用 `rustls` feature（3.x feature 名是 `rustls` 而非 `tls`），避免引入系统 TLS 依赖。同时修复了若干编译/运行错误：`commands.rs::install_kernel` 把 `data_dir` / `version` 在送入 `spawn_blocking` 闭包前先 `clone`，这样 join 后仍能用它们做后续 `set_active`/自动启动；`commands.rs` 给 `Channel<String>::send` 传 `String` 而非 `&str`；`kernel.rs::port_open` 直接构造 `SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127,0,0,1), port))`，因为 `(&str, u16)` 不实现 `Into<SocketAddr>`；`releases.rs` 用 ureq 3.x 的 `.header(...)`（不是 `.set`）并给每个 `map_err` 闭包显式标注 ureq 错误类型；`node.rs::parse_version` 三个数字段都用 `parse::<u32>()`；`kernel.rs::start` 把 `port.to_string()` 提前为 `String` 再传入 `cmd.arg(...)`；`releases.rs::parse_atom` 把切片偏移始终锚定在原 buffer 上，避免多字节 UTF-8 标题把 `&str` 切片切到字符边界之间导致 panic。代码 review 轮的四项健壮性修复：`run_pnpm` 改用 `recv_timeout(10s)` 接收，依赖解析静默期发「已进行 N 秒」心跳而不是看起来卡死；内核 pid 在 spawn 时写入 `<data_dir>/kernel.pid`，`stop_kernel` 与 `RunEvent::Exit` 钩子都会在端口仍被占用时回退到 `kill_pid`（先用 ps 校验命令行身份防 pid 重用误杀），崩溃后遗留的孤儿内核也能回收；`write_active` 先写 `active.txt.tmp` 再 rename、删除时容忍文件不存在，崩溃不会截断活动指针；`rotate_install_logs` 在每次安装前只保留最近 9 份安装日志。本地环境没有 Rust 与 Node 工具链，因此 `cargo build` 与 Tauri 装配由 CI 中的 `desktop-release` workflow 验证；更新菜单的发布列表在装有 Node 并构建出外壳后，于应用内直接对真实 npm registry（以及 GitHub fallback）完整跑一遍。
