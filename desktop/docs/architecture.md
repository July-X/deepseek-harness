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

- `commands.rs`：Tauri 命令层。长任务用 `spawn_blocking` + `tauri::ipc::Channel` 向 UI 推进度事件。
- `kernel.rs`：内核安装、active 指针、启动 / 停止、端口探测；详见下文「内核生命周期」。
- `plugins.rs`：社区插件的中央库、内核物化、profile 接线、更新检查、社区目录；实现规则见 [plugin-internals.md](plugin-internals.md)，设计层见 [plugin-management.md](plugin-management.md)。
- `releases.rs`：npm registry 全量版本 + dist-tags；registry 不可达时回退 GitHub Releases API 与 Atom feed。
- `node.rs`：Node 检测（显式配置 → PATH → nvm 管理的 Node：macOS/Linux `$NVM_DIR/versions/node/<v>/bin/node` 跟随 `alias/default` 链，Windows `%NVM_SYMLINK%` 与 `%NVM_HOME%/v*/node.exe` → 常见系统位置）、engines 校验（`^22.19 || >=24`）、pnpm/npm 解析（显式配置 → node 同目录 → PATH）；空结果文案按「完全没有 Node」与「Node 版本太老」分别给出可操作的多路径（nvm/fnm/volta、brew/winget/apt、官方安装包）。
- `settings.rs`：`settings.json` 平铺结构（`node_path` / `pnpm_path` / `port`），serde default 兼容缺字段。
- `process.rs`：所有 GUI 子进程的 `quiet()`（CREATE_NO_WINDOW）+ `command_with_path()`（一次性 sibling，盖上 `env::merged_path()`）出口。
- `updater.rs`：`tauri-plugin-updater` 包装，启动 3 秒后后台检查并 emit `shell-update-available`。
- `lib.rs`：装配 + `setup()` 取目录（必须走 `kernel::data_dir`）+ `RunEvent::Exit` 兜底回收内核进程组。

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