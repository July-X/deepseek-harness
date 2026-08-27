# Agent Note: dsh-desktop first-install log directory and merged-PATH tool detection

Status: implemented
Archived: 2026-08-28

[English](2026-08-24-desktop-first-install-log-dir-and-detection-merged-path.md) | 中文

## Problem

一台 nvm-windows 机器（Node 检测正常、pnpm 确实未安装）上，GUI 启动的 Windows 外壳首次安装内核即报错「无法运行 npm 以自动安装 pnpm：系统找不到指定的路径 (os error 3)」。这条信息背后叠加了两个 bug，恢复路径上还有一个潜在 bug。

**npm 还没运行，开日志就失败了。** `ensure_pnpm` 把 `<data_dir>/logs/` 下的日志路径交给 `process::run_with_progress`，但全新数据目录里没有任何东西创建过 `logs/`——`install_version` 和 `kernel::start` 都要在流程更后面才创建它。`OpenOptions::open` 以 `NotFound`（Windows `ERROR_PATH_NOT_FOUND`，os error 3）失败，而外层文案把锅甩给了 npm。故障机器上的佐证：数据目录里没有任何 `pnpm-install-*.log` 文件——若是 spawn 失败，至少会留下一个（空）日志。

**检测仍在扫原始进程 PATH。** [此前的 Windows shim/PATH 修复](2026-08-22-desktop-windows-pnpm-npm-resolve-and-path.zh.md) 把用户 PATH（`HKCU\Environment\Path`）合并进了每个 spawn 出来的子进程，但 `resolve_pnpm`、`find_npm`、`from_path` 仍扫 `std::env::var_os("PATH")`。GUI 子系统进程只继承系统 PATH，所以用户早已装在 `%AppData%\npm` 下的 `pnpm.cmd` 对它不可见，外壳不必要地走进了自动安装分支。

**潜在问题：prefix 回退在 Windows 上根本跑不起来。** `npm_prefix` 直接 spawn `npm`，而 Windows 上的 npm 是 `.cmd` 批处理 shim，CreateProcess 无法执行批处理文件——恰恰是最需要自动安装的用户走不到装后 prefix 探测。

## Decision

- `run_with_progress` 打开日志文件前先创建其父目录；契约改为调用方永远不需要预建日志目录。
- `node.rs` 的全部 PATH 扫描统一走 `path_dirs()` 助手，基于 `env::merged_path()`——检测看到的 PATH 与外壳盖给子进程的 PATH 完全一致。
- `npm_prefix` 改用新的 `process::script_output`：`spawn` 的一次性版本，`.cmd` shim 经 `%ComSpec% /C` 路由，并盖合并 PATH（工具自身目录前置）。
- npm spawn 失败的报错信息现在附上完整日志路径，符合桌面壳「报错必须指明日志」的规则。

## Alternatives considered

- **在 `promise_pnpm` 或应用 setup 里创建 `logs/`** —— 只修好一个调用方，其余每个 `run_with_progress` 调用方仍留着同样的隐患；由 helper 自己负责开日志才是唯一正确的位置。
- **检测扫描前展开 `REG_EXPAND_SZ` 的 `%VAR%` 引用** —— 为一种罕见的存储形式手写环境变量展开；未展开的条目只是 `is_file` 落空、自然跳过，与此前笔记记录的子进程 PATH 行为一致。
- **检测维持进程 PATH，让用户去设置里填 `pnpm_path`** —— 等于让每个 pnpm 装在用户 PATH 下的 GUI 用户都手动改一次 JSON，而外壳维护合并 PATH 本来就是为了解决这件事。

## Consequences

在报告问题的机器类型上（GUI 启动的 Windows 外壳，pnpm 缺失或只在用户 PATH 下），首次安装现在要么直接找到已有的 `pnpm.cmd`，要么真正跑起 `npm install -g pnpm`，完整输出落盘 `logs/pnpm-install-*.log`；nvm-windows 下全局 prefix 是用户可写的版本目录，无需提权，新装的 shim 立刻能被 `node_dir` 探测到。误导性的「无法运行 npm」包装此后只会对应真实的 npm 启动失败，且指明要看的日志。验证：新增 `process::tests::run_with_progress_creates_missing_log_directory` 回归测试；`cargo check`、`cargo clippy --all-targets`（零警告）、`cargo fmt` 与 lib 测试套件通过（两个 `plugins::tests` 的 Windows 失败在干净树上可复现，为既有问题、与本次无关）。合并 PATH 仍是每进程缓存一次，会话中途新装的工具要重启后才能被检测到——与此前的笔记记录的限制一致。
