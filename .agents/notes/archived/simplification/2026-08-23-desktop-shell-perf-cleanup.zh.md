# Agent Note: 桌面端性能修复与代码精简

Status: implemented
Archived: 2026-08-28

[English](2026-08-23-desktop-shell-perf-cleanup.md) | 中文

## 问题

Windows 构建体感卡顿，还有残留的终端窗口闪屏。审计在 IPC 两侧都找到了根因：同步 Tauri 命令在主线程做重活、一个 console 子系统进程漏了 `CREATE_NO_WINDOW`、以及一个无条件重写 DOM 的 UI 轮询循环。

## 决策

**所有阻塞命令都走 `spawn_blocking`。** Tauri 的非 async 命令在主线程执行，因此 `start_kernel`（最坏要跑完整 `pnpm install`)、`stop_kernel`、`activate_version`、`remove_version`、`fetch_releases`、`get_status`、`plugin_status` 全部改成 async 命令并把活交给阻塞线程池，沿用现有 `plugin_catalog` 模式。`State` 无法 move 进闭包，所以处理器带 `AppHandle`，在闭包内重新 `app.state::<AppState>()`。

**`kernel::stop` 用 `try_wait` 轮询，一秒预算后 SIGKILL。** 旧代码在 SIGTERM 后直接阻塞在 `wait()`，后面的 SIGKILL 永远不可达——内核一旦不响应，「关闭工作台」和应用退出都会永久卡住。轮询循环照抄同文件的 `kill_pid`。

**`reg.exe` 的 PATH 探测走 `quiet()`。** 它是 `c22e3efa84` 之后唯一漏掉 `CREATE_NO_WINDOW` 的 console 子系统 spawn；该文件自己的注释本来就写着需要。

**轮询有门控且最小化写入。** 2.5 秒状态轮询在 `document.hidden` 时跳过，`visibilitychange` 恢复时立即刷一次；`setText` helper 跳过同值的 `textContent` 写入；删掉 `StatusView.kernel_log`（前端从不读它，日志走 `get_kernel_log`)。`promise_pnpm` 改用缓存的 `NodeInfo`，不再每次操作重新 spawn `node --version`。

**精简贴着被改动的代码做。** 共享 pnpm 参数（`PNPM_REPORTER`、`PNPM_NO_STRICT_DEP_BUILDS`)、`pnpm_spawn_err`、`http_get_string`/`http_get_bytes`，以及一个 `run_plugin_command` 公共 runner 替换了五个近乎相同的插件命令；UI 侧新增 `el`/`mkBtn`/`armConfirm` helper，收编了约 13 处重复的 DOM 构建块和两份两段式删除确认。

## 备选方案

**内核状态改事件推送替代轮询。** 暂不采纳：内核是桌面端只能观察的外部进程，存活探测无论如何都需要；把轮询挂上门控并移出主线程已经消掉了实测开销，不必改协议。

**对 `plugins.rs` 做更深重构。** 否决：那里剩余的重复（install/update 尾部、semver 解析）留到下次插件机制改动时顺手提取，比单独重构更划算。

## 后果

- `get_status` 和 `plugin_status` 的命令签名变为 `Result<_, String>`；成功 payload 形状不变，reject 只在阻塞线程本身失败（panic）时发生——这在以前无法被观察到。
- `withPluginProgress` 的 `fail`/`done` 标签现在有兜底，缺标签的调用方（如 `updatePlugin`）不会再 toast 出 `undefined`。
