# dsh-desktop 架构

桌面壳的模块布局、数据流与数据目录约定。约定性约束（必须照做）见 [AGENTS.md](../AGENTS.md)。

## 模块

```
ui/app.js + ui/plugins.js ──invoke(Channel)──▶ commands.rs ──▶ kernel.rs / plugins.rs ──▶ pnpm/git/tar 子进程
                                   │              │
                              settings.rs    releases.rs（npm registry → GitHub 回退）
                                   │
              ~/.dsh/desktop/{settings.json, kernels/, logs/, active.txt} + ~/.dsh/plugins/
```

- `commands.rs`：Tauri 命令层。长任务用 `spawn_blocking` + `tauri::ipc::Channel` 向 UI 推进度事件。`open_official_chat` 在独立线程里 `WebviewWindowBuilder::new(...).label("official-chat").title("DeepSeek 官方对话")`，固定 `OFFICIAL_CHAT_URL`（`https://chat.deepseek.com`），构造时不再覆盖 user-agent——WebView2 引擎本身就是真实的桌面版 Edge，原生 UA、`Sec-CH-UA` 客户端提示与 `navigator.userAgentData` 天然一致；此前把 UA 改写成 Chrome 反而制造了「HTTP 层报 Edge、JS 层报 Chrome」的矛盾，正是环境检测的特征。配合 `.incognito(true)`（干净的会话，不共享 cookie）；再调 `.additional_browser_args(OFFICIAL_CHAT_BROWSER_ARGS)` 抑制 Chromium 自报的 `navigator.webdriver = true`，同时重述 wry 默认禁用的 `msWebOOUI` / `msPdfOOUI` / `msSmartScreenProtection`——传入 browser args 会整体替换 wry 默认值，漏掉这三项 WebView2 就会重新弹出 SmartScreen 安全提醒与 Edge 专属 UI；该参数仅 WebView2 后端消费，macOS / Linux 构建忽略——且 WebView2 要求同一 user-data 目录上的环境参数完全一致，面板与工作台已在默认目录用默认参数建好环境，所以此窗口经 `.data_directory` 固定到专属目录 `<data_dir>/webview-official-chat`，否则环境创建失败、窗口无法出现；之后按 `pullstring-launcher.js` → `titlebar-pulse.js` → `chat-fingerprint.js` 的固定顺序注入三个 `initialization_script`，最后再 `.build()`——第一条先捕获真实 `window.__TAURI__` 到闭包变量 `__DSH_TAURI_REF__`，第三条再把全局替换为 neutered Proxy，所以拉绳挂件在 Proxy 生效后仍能拉起管理面板。复用既有窗口（`get_webview_window` + `set_focus`），不设 `closable(false)`——第三方 origin 的窗口不持有内核会话，OS 关闭按钮应保持有效。`open_official_chat` 是 `async` 命令，builder 结果通过 `std::sync::mpsc::channel` 回传、由 `tauri::async_runtime::spawn_blocking` 接收，命令只在线程里 `Result<WebviewWindow, _>` 真正落地之后才 `Ok(())`。配套的 `close_official_chat` 在窗口未注册时返回错误，存在的窗口走 `destroy()`；面板按钮在 `StatusView.official_chat_open` 的下一次 2.5s 轮询里把按钮文案从「打开官方对话」翻为「关闭官方对话」。
- `kernel.rs`：内核安装、active 指针、启动 / 停止、端口探测；详见下文「内核生命周期」。
- `plugins.rs`：社区插件的中央库、内核物化、profile 接线、更新检查、社区目录；实现规则见 [plugin-internals.md](plugin-internals.md)，设计层见 [plugin-management.md](plugin-management.md)。
- `releases.rs`：npm registry 全量版本 + dist-tags；registry 不可达时回退 GitHub Releases API 与 Atom feed。
- `node.rs`：Node 检测（显式配置 → PATH → nvm 管理的 Node：macOS/Linux `$NVM_DIR/versions/node/<v>/bin/node` 跟随 `alias/default` 链，Windows `%NVM_SYMLINK%` 与 `%NVM_HOME%/v*/node.exe` → 常见系统位置）、engines 校验（`^22.19 || >=24`）、pnpm/npm 解析（显式配置 → node 同目录 → PATH）；空结果文案按「完全没有 Node」与「Node 版本太老」分别给出可操作的多路径（nvm/fnm/volta、brew/winget/apt、官方安装包）。
- `settings.rs`：`settings.json` 平铺结构（`node_path` / `pnpm_path` / `port`），serde default 兼容缺字段。
- `process.rs`：所有 GUI 子进程的 `quiet()`（CREATE_NO_WINDOW）+ `command_with_path()`（一次性 sibling，盖上 `env::merged_path()`）出口。
- `updater.rs`：`tauri-plugin-updater` 包装，启动 3 秒后后台检查并 emit `shell-update-available`。
- `lib.rs`：装配 + `setup()` 取目录（必须走 `kernel::data_dir`）+ `RunEvent::Exit` 兜底回收内核进程组。`harness` 与 `official-chat` 两个 webview 窗口通过 `capabilities/harness-remote.json` / `capabilities/official-chat-remote.json` 分别绑定 ACL；两个文件都只授权 `allow-focus-main-shell`——拉绳挂件只需要这一条 IPC 命令，URL 都精确钉死（`http://127.0.0.1:*` / `https://chat.deepseek.com/*`，不开通 wildcard 域名）。

## 内核生命周期

- 安装：在 `<data_dir>/kernels/<version>/` 写最小 stub `package.json` 后执行 `pnpm add --prefix … --ignore-workspace --config.node-linker=hoisted --reporter=append-only @deepseek-ai/dsh@<version>`。
- `node-linker=hoisted` 保证 `node_modules` 扁平，内核入口固定为 `node_modules/@deepseek-ai/dsh/lib/bin.js`（`kernel::KERNEL_BIN_REL`）；改布局必须同步该常量与 `start()`。
- `run_pnpm` 把 stdout/stderr 各用一个 drain 线程读入 mpsc channel，安装线程逐行回调 `on_progress` 并落盘日志——不要把两个管道放在同一线程顺序读取（会因管道缓冲区满而死锁）。

## 数据目录

外壳全部状态位于 `<dsh_home>/desktop/`（release build）或 `<dsh_home>/desktop-dev/`（debug build `tauri dev`），由 `kernel::data_dir` 解析并在启动时创建。子结构：`kernels/<版本>/`、`logs/`、`settings.json`、`active.txt`、`kernel.pid`。

启动时 `setup()` 在 stderr 打印 `dsh-desktop: data_dir = <path> (build: dev|release)`，让用户一眼确认当前进程用的是哪个目录。

### 优先级（`kernel::data_dir`）

1. `DSH_DESKTOP_DATA_DIR` 环境变量——完全覆盖目录路径（用于在外部盘上测试等场景）
2. `<DSH_HOME 或 ~/.dsh>/<SHELL_SUBDIR>/`——`SHELL_SUBDIR` 在 release 是 `desktop`、debug 是 `desktop-dev`
3. `app_data_dir()`（OS app-data 目录）作为只读 dsh home 的 fallback

### 为什么 dev 和 release 用不同目录

`settings.json`（端口配置）、`active.txt`（当前激活版本）、`kernel.pid`（运行中内核的 PID）、`kernels/<版本>/`（安装的内核）、loopback 端口都是**共享资源**。一个开发者同时跑 `tauri dev` 和已装的 release shell 时，两个实例会互相争端口（`port_open` 拒绝启动）、互相 kill（任意一方点"关闭工作台"就把对方的内核杀了）、互相覆盖 `active.txt` 和 `settings.json`。分目录 + 错位端口（debug 3091 / release 3090）让两边完全互不读对方的 state——dev 可以放心改端口、切内核、看 log，不会污染 release shell 的视图。

### 端口（`kernel::DEFAULT_PORT`）

- debug build：3091（release 默认 3090 + 1）
- release build：3090

`Settings::default()` 的 port 在 `settings.json` 缺失时用 `kernel::DEFAULT_PORT`；用户保存过的 port 优先。

## 窗口

- `main`（管理面板）：`tauri.conf.json` 里配置为主窗口；加载 `ui/` 静态资源，`capabilities/default.json` 拥有全部本地命令权限。
- `harness`（工作台）：`open_harness` 在新 OS 线程里 `WebviewWindowBuilder::new(...).label("harness")`，加载 `http://127.0.0.1:<port>`；`closable(false)` 防止误关丢内核会话，由 `stop_kernel` 用 `destroy()` 主动回收。`capabilities/harness-remote.json` 仅授权 `allow-focus-main-shell`，URL 锁 `http://127.0.0.1:*`。
- `official-chat`（DeepSeek 官方对话）：`open_official_chat` 在新 OS 线程里 `WebviewWindowBuilder::new(...).label("official-chat")`，加载 `https://chat.deepseek.com`；构造时不覆盖 user-agent（诚实呈现桌面版 Edge，UA / 客户端提示 / `userAgentData` 一致）、附 `.incognito(true)`（干净的会话）与 `.additional_browser_args(OFFICIAL_CHAT_BROWSER_ARGS)`（重述 wry 默认禁用项并叠加 `AutomationControlled` 等开关：Chromium 不自报 `navigator.webdriver = true`，也不弹 SmartScreen 安全提醒；参数仅 WebView2 消费，其他平台忽略），让 chat.deepseek.com 的环境检查把它当作普通桌面浏览器；不设 `closable(false)`，OS 关闭按钮正常工作，重复点击复用既有窗口并 `set_focus`，已开时通过 `close_official_chat` 主动销毁。`capabilities/official-chat-remote.json` 仅授权 `allow-focus-main-shell`，URL 锁 `https://chat.deepseek.com/*`。`StatusView.official_chat_open` 由 `get_status` 在 `spawn_blocking` 内同步读取，作为面板按钮切换「打开官方对话」/「关闭官方对话」标签的信号。

`titlebar-pulse.js` / `pullstring-launcher.js` / `chat-fingerprint.js` 由外壳通过 `WebviewWindowBuilder::initialization_script` 按固定顺序注入到 webview；`official-chat` 三个全用，`harness` 只用前两个：

- `pullstring-launcher.js`（`official-chat` 注入顺序的第 1 位）：在页面左上角挂一个灯泡拉绳。`chat.deepseek.com` 时贴 `left:12px`（贴近页面自己的左上角 chrome）、cord 为 `#4D6BFE`；workbench 时 `left:212px`（侧栏折叠按钮旁）、cord 为 `#609926`。在 IIFE 顶部就把 `window.__TAURI__` 捕获到闭包变量 `__DSH_TAURI_REF__`，拉一下调用 `focus_main_shell` 时从闭包取真实 IPC 桥，所以 `chat-fingerprint.js` 后续把全局替换成 neutered Proxy 之后拉绳挂件仍然能拉起管理面板；`focus_main_shell` 在两个 capability 里是唯一授权的命令。
- `titlebar-pulse.js`（`official-chat` 注入顺序的第 2 位）：接管 chrome-row 顶部条带。`location.hostname` 命中 `chat.deepseek.com` 时使用 DeepSeek 官方蓝 `#4D6BFE`（rgb 77,107,254）；否则用 Gitea 绿 `#609926`（rgb 96,152,38）。两个 sweep 周期相同（6.01s），半周期偏移。workbench 页面自带 `<body><div data-titlebar-pulse="2">`，chat 页面没有——脚本用 `ensureSecondBar()` 在缺失时补上。
- `chat-fingerprint.js`（`official-chat` 注入顺序的第 3 位，**必须最后**）：只清除嵌入式痕迹，不再伪造浏览器指纹——把 `navigator.webdriver` 钉在 `false`（正常浏览器的值），并删除 `__TAURI__` / `__TAURI_INTERNALS__` / `__TAURI_METADATA__` / `__TAURI_IPC__` 全局（正常浏览器里它们根本不存在；暴露任何形式的 Proxy 都等于自报嵌入式身份）。其余表面保持真实：引擎是货真价实的桌面版 Edge，用普通 JS 对象冒充 `userAgentData` / plugins 等反而会被原生类检查识破。注入顺序出错会让它先于 `pullstring-launcher.js` 跑，`__TAURI__` 被删之后拉绳挂件就再也拉不起管理面板了。
- 三个脚本顶部都有 `if (window.top !== window.self) return` 顶帧守卫，避免 Tauri 在每个 iframe 都执行初始化脚本时挂出多份拉绳 / 条带 / 指纹 stub。