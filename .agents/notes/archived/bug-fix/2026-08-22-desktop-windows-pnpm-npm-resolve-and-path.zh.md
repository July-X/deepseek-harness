# Agent Note: dsh-desktop 现在能找到 Windows 的 pnpm/npm shim，并且继承用户 PATH

Status: implemented
Archived: 2026-08-28

[English](2026-08-22-desktop-windows-pnpm-npm-resolve-and-path.md) | 中文

## 问题

两个 Windows 专属的故障叠加在一起，让桌面应用默认情况下的自动安装路径走不通。两个问题都在 `node::ensure_pnpm`（内核和插件安装的引导路径）里爆发，用户看到的就是"未检测到 pnpm，正在通过 npm 自动安装…安装失败：系统找不到指定的路径"这串报错。

**查找形状不对。** `resolve_pnpm` 和 `find_npm` 遍历 PATH 时找的是 `<tool>.exe`。但 Windows 上 `npm install -g` 装的 Node 系工具是 `.cmd` shim，落在用户的 npm prefix（`%AppData%\npm\pnpm.cmd`，永远不是 `.pnpm.exe`）下；用 portable 布局时 `npm` 自己也是同样情况。这套查找逻辑对每一个用户 prefix 装的工具都会漏掉，只在工具恰好躺在 `node.exe` 同目录这种少见情况下才工作——`pnpm` 几乎不满足这个条件。

**GUI 进程丢用户 PATH。** Tauri 用 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`，运行时是 GUI subsystem 二进制。GUI subsystem 进程启动时只继承系统 PATH block——`HKEY_CURRENT_USER\Environment\Path`（用户安装 npm/pnpm 时写入的值）会被 Explorer 和其他交互式宿主合并进系统 PATH，但从来不会进 `CreateProcess` 交给 GUI 子进程的那个 PATH block。系统装的 `node.exe` 还能找到，用户之后装的任何东西都找不到。release 构建 100% 复现；debug 构建继承父进程 PATH，只在某些情况下撞到。

合在一起就是：`resolve_pnpm` 返回 None（PATH 上没有 `.exe`）→ `ensure_pnpm` 落进 `npm install -g pnpm` 分支 → `process::spawn` 也找不到 `npm.cmd`（用户 PATH 缺失）→ 用户看到包了一层中文错误信息的 "os error 3"，根本看不出来真正原因。

## 决策

**一个 Windows 感知的帮助函数，同时服务 `resolve_pnpm` 和 `find_npm`。** `node::which_in_dir(name, dir)` 在 Windows 上对每个目录按顺序试三个候选（`<name>.cmd`、`<name>.exe`、`<name>`），Unix 上只试 `<name>`。`.cmd` 优先的顺序匹配 `npm install -g` 装出来的实际形态；`.exe` 保住独立安装场景；裸名兜住极少数 PATH 段已经带扩展名的边角。`from_path`（找 `node`）刻意保持只查 `.exe`——`node` 本身从来不是 shim。

**Settings 加 `npm_path` 槽。** `Settings::npm_path: Option<String>` 被 `find_npm` 第一档检查，对齐已有的 `pnpm_path`。默认 `None`；现有 `settings.json` 反序列化无变化，因为字段标了 `#[serde(default)]`。UI 暂时还不暴露这个槽，所以现在只能直接编辑 `~/.dsh/desktop/settings.json`——这跟 `pnpm_path` 原本的设计一致，是手动的应急通道。

**每个 spawn 的子进程都拿到合并后的 PATH。** 新模块 `desktop/src-tauri/src/env.rs` 用 `reg.exe query HKCU\Environment /v Path` 启动时读一次 `HKEY_CURRENT_USER\Environment\Path`，解析 `REG_SZ` / `REG_EXPAND_SZ` 的值，去重合并继承下来的系统 PATH（大小写不敏感，用户项优先，匹配 Explorer 拼起来的顺序），结果缓存在 `OnceLock` 里。`process::spawn` 在每个 `Command` 上盖 `cmd.env("PATH", env::merged_path())`，Windows 和 Unix 都走同一段代码；Unix 直接返回父进程 PATH 不动。`reg.exe` 走固定路径 `C:\Windows\System32\reg.exe`，stdio 全 pipe 加 `quiet()`，不会闪控制台；`OnceLock` 初始化把读操作挪出 spawn 热路径。

## 后果

已交付：`which_in_dir` 帮助函数加 5 个测试；`parse_reg_path` 加 `merge_paths` 加 5 个测试，覆盖 `reg query` 输出格式和去重 / 用户优先顺序；`npm_path` settings 槽沿用 serde-default 兼容旧配置；`env::merged_path()` 应用到所有 `process::spawn` 调用（内核安装、插件库依赖、profile 接线、git 来源插件 `prepare` 构建，以及新加的 `npm install -g pnpm` fallback）。shell 现在能定位用户 prefix 里的 `pnpm.cmd`，所有子进程也都继承了用户 PATH——从桌面启动的 Tauri 构建跟 `cargo tauri dev` 在中国网络下安装行为一致。`find_npm` 的失败分支（"未检测到 pnpm，也未找到可用的 npm"）仍然在配置路径、node 同目录、PATH 都没有 npm/pnpm 二进制时正确触发——这次改动纯粹是加性的。

已知限制。注册表读发生在进程启动时；如果用户在会话中间又装了新工具，缓存的 PATH 不会包含，要重启才能看见（跟 `commands.rs` 里 `node_cache` 的现成模式一致）。`npm_path` 还没接到 UI 上；现在只能编辑 `settings.json` 设。`parse_reg_path` 依赖 `reg.exe` 默认的文本布局，将来 Windows 如果改了列格式需要跟一个补丁。PATH 合并不展开 `REG_EXPAND_SZ` 的 `%VAR%` 引用，子进程原样继承，Windows 在查找时自己展开——跟 `cmd.exe` 自己的行为对齐。

## 备选方案

- **保持 `.exe` 唯一，让用户去设 `pnpm_path` / `npm_path`**——能缓解症状，但要用这套配置的位置（`ensure_pnpm` 的 fallback）恰好是还没收集到用户配置的入口，用户在能装任何东西之前就被迫编辑 JSON。
- **用 `which` crate**——为 30 行文件查找拉一个依赖，并且它对 `PATHEXT` 顺序的处理跟我们想要的不一致（它按目录逐个试扩展名，这是对的，但默认的 `is_absolute()` 过滤会把 PATH 上的相对条目都拒掉）。
- **把 shell 在 Windows 上切成 console subsystem**——放弃 `windows_subsystem = "windows"`，每次启动都闪控制台窗口。不行。
- **启动时用 `winreg` crate 读用户 PATH**——加依赖还把读路径搞复杂；`reg.exe` 落在固定 Windows 路径，根本不用 crate，每个 Windows shell 编辑器 UI 都用同样方式读这个值。
- **spawn 时对 `REG_EXPAND_SZ` 做 `expand_env`**——能让 `merged_path()` 拿真路径而不是 `%USERPROFILE%` 风格的引用；先不做，子 shell 查找时已经会展开，跟 `cmd.exe` 自己的行为对齐。
- **在 UI 里跟 port / profile 一起暴露 `npm_path`**——改动范围大（HTML、JS、命令处理、校验），不在修这次安装失败的范围内；JSON 编辑路径跟 `pnpm_path` 现有待遇一致。
