# Agent Note：dsh-desktop 直接调起的 git/tar 子进程也继承合并后的 PATH

状态：已实现

[English](2026-09-01-desktop-direct-git-tar-merged-path.md) | 中文

## 问题

前面的修复让 Windows GUI release 构建在每个 `process::spawn` 点（内核安装、插件库依赖、profile 接线、npm/pnpm fallback、git 来源插件的 `prepare` 构建）都拿到用户 PATH。但漏掉了一个分支：插件库里 git 和 tar 的调用是短生命周期的 helper，没有走 `process::spawn`，而是直接用 `Command::new("git")` / `Command::new("tar")` 拼出 `Command`。这些子进程继承的是 GUI shell 启动时的 PATH 块——Windows 上只有系统 PATH——所以漏掉了用户装在 `HKCU\Environment\Path` 里的任何东西。

Windows 上的具体症状：用户用 Git for Windows 标准安装器（会把 `C:\Program Files\Git\cmd\` 写进**用户** PATH，不是系统 PATH）装了 git，打开 `tauri build` 出的桌面壳，试着装一个 git 来源的插件。`fetch_git` 里的 `Command::new("git")` 探测失败，错误以 `未找到 git（git 来源的插件需要 git；请先安装 git）` 的形式冒出来，安装还没进到 `git ls-remote` 或 `git clone` 就停了。同样的形态会命中以后任何用户-PATH-only 的工具；`tar` 现在恰好能用，是因为 Windows 10+ 把 `bsdtar` 装在 `C:\Windows\System32\tar.exe`（系统 PATH）——但这种运气不能依赖。

pnpm 那边的注释（`fix(desktop): prepend pnpm's bin dir to child PATH for plugin builds`，75046c1a9d）和 Windows 注册表 PATH 合并（bfde8dd884）搞定了 pnpm/npm 和 prepare-build 流程；它们没有覆盖 `plugins.rs` 里的 `Command::new` 调用，因为这些路径根本不经过 `process::spawn`。

## 决策

**新增一次性 helper `process::command_with_path(program)`。** 它构造 `Command` 之后立刻盖上 `cmd.env("PATH", env::merged_path())` 再返回，让 shell 里每个直接调起的外部工具都从一个入口走，拿到 `process::spawn` 已经给长生命周期 helper 准备的同一份合并 PATH。Unix 上合并是透传，Windows 上带回 GUI subsystem 会丢的用户 PATH。helper 接 `S: AsRef<OsStr>`，调用方可以直接传 `"git"`、`"tar"` 或任何将来的工具名字，原来的 `Command::new` 用法不受影响。

**`plugins.rs` 里每个直接调起的外部工具现在都走这个 helper。** 三处迁移：

- `run_capture`（`git_latest_tag` 用它调 `git ls-remote`）。
- `fetch_git` 的 `git --version` 探测。
- `fetch_git` 的 `git clone --depth 1` 命令。
- `extract_tarball` 的 `tar -xzf … -C …` 调用。

`pnpm install`、`node`（内核）、`npm install -g pnpm` 已经走 `process::spawn`，没动。`plugins.rs` 里的 `Command` 和 `Stdio` import 收紧到只剩 `Stdio`（`git clone` 那一处还要 `Stdio::null()` 让 clone 静默）；这个文件里没有别的代码再直接构造 `Command`。

**新增的 helper 用真实 spawn 验证，不是 Debug 打印偷懒。** Rust 的 `Command` Debug 输出只显示 program 和 args（env 项藏在不透明的内部表里），所以只检查 `format!("{cmd:?}")` 的单测抓不到漏掉 `env` 的错误。新增的测试通过 helper 起 `cmd.exe /C "echo %PATH%"`（Windows）或 `/bin/sh -c 'echo "$PATH"'`（Unix），断言子进程回显的 PATH 跟 `env::merged_path()` 逐字节一致。这才能证明 helper 真的把 PATH 盖到子进程头上，而不是「看起来」盖了。

**`extract_tarball` 现在预创建目标目录，并把 stderr 拼进错误信息。** 上面那条用户 PATH 修复让 Windows 上的 `tar.exe` 能找到，但同一波用户在它后面又撞了第二个问题：`tar -xzf … -C <dest>` 在 `<dest>` 不存在时直接退出码 1，stderr 是 `could not chdir to`。GNU tar 会按需建目录，Windows 10+ 自带的 bsdtar（在 `C:\Windows\System32\tar.exe`）不会；而原本的错误信息（`退出码 Some(1)`）把真实原因藏起来了。修复就是 spawn 之前先 `fs::create_dir_all(dest)`，再把 stderr pipe 回来——下一个失败会直接告诉用户 bsdtar 的诊断（坏归档、MAX_PATH 超限、权限拒绝……），不再是个没法操作的退出码。stdout 丢掉，因为 bsdtar 每解开一个路径都会打一行，那种噪音不应该出现在安装日志里。

**同一笔改动里把 `process::tests` 里早就存在的跨平台测试 bug 也修了。** 那些 `merge_extra_path_*` 测试硬编码了 Unix 的 `:` 作为分隔符；Windows 上 helper 实际产出 `;`，所以在 Windows 上跑 `cargo test --lib` 会失败——而且失败先于这次改动发生。每个测试现在从 `cfg!(windows)` 推导它期望的分隔符。这些失败之前没有挡住 CI（Windows CI 通道只覆盖 `wine-windows-gates.sh`，那是内核包的包），但开发者在 Windows 上跑 lib 测试会撞上，修复每条断言也就两行。

## 后果

已交付：`process::command_with_path` helper + 1 个 spawn 测试；`plugins.rs` 把 `run_capture`、`fetch_git` 的探测、`fetch_git` 的 clone、`extract_tarball` 都改走 helper；`extract_tarball` 现在 `mkdir -p` 自己的目标目录并把 tar 的 stderr 转写到错误信息里；helper 是未来 shell 里直接调工具的官方入口；`merge_extra_path_*` 在 Windows 上的既存测试失败一并修好，让它们跟随 `cfg!(windows)`。

配合之前的修复，这次补齐了 shell 在 Windows 上对所有 spawn 出去的工具的用户 PATH 路径：

- 用户 npm prefix 下的 pnpm/npm shim → `process::spawn`（已通过 `env::merged_path` 修好）。
- `git` / `tar` / 将来直接调的工具 → `process::command_with_path`（新增）。
- Windows bsdtar 的 `tar -C` 怪癖 → spawn 前 `fs::create_dir_all`（新增）。
- `node`（内核入口）和 `npm install -g pnpm` fallback 已经通过同一条 `process::spawn` / `node::ensure_pnpm` 路径拿到合并 PATH。

Windows 上标准装 Git for Windows（仅用户 PATH）的用户，现在能从 `tauri build` 出的 release 安装 npm 和 git 来源两种插件。git 真的没装时，原本的 "未找到 git" 错误还是会正确触发——探测还是调 `git --version`，helper 只是改了它查找时看的 PATH。npm tarball 解包不再在 staging 目录还没建的时候甩出那行没法操作的 `退出码 Some(1)`；下次再失败（坏归档、路径过长、ACL 拒绝），用户看到的是 bsdtar 真实报出来的诊断，不再只是个退出码。

`plugins.rs` 里还有两个 Windows-only 的旧测试失败（`computes_relative_paths` 和 `materialize_link_then_copy`）没动：它们跟这次修复无关，属于测试 harness 自己的 bug，应该在独立的提交里改，避免这次 diff 越界。

已知限制。PATH 缓存在进程启动时（`OnceLock`）；用户在 shell 运行时新装 git 还是要重启 shell 才看得到新条目（跟现有 `node_cache` 一致）。`command_with_path` 只设 `PATH`；将来若有工具需要别的环境变量（`GIT_TERMINAL_PROMPT`、`CARGO_TERM_COLOR` 之类），调用点还得自己加。shell 不在这里加 `extra_path_dirs`——那条路径留给 `process::spawn`，因为目前已知的消费者只有 pnpm 的 bin 目录（为了生命周期脚本能解析 `node`）。

## 备选方案

- **改让 `git` 和 `tar` 走 `process::spawn`。** 否决：这些都是一锤子命令，不需要双流 drain 或心跳；为了单一好处给 `spawn` 加 `fire-and-forget: true` 旋钮会撑大它的契约。一次性 helper 几行就够，复用同一个 `env::merged_path`。
- **包一层 `Command::new`，搞个新的 `Command` 包装类型。** 否决：所有已经写成 `Command::new(...)` 的地方都要学新类型，而 `Command` 的 API 表面太大，包装类型在第一次非平凡组合里就会漏风。一个 free function 够了。
- **在文档里写「用 `process::spawn` 别用 `Command::new`」就算了。** 否决，因为 `process::spawn` 强制要 log 路径和进度回调；`tar -xzf` 和 `git ls-remote` 都不想要日志文件、也不想要心跳。为了形式一致强迫它们走 `process::spawn` 是为了仪式感而仪式。
- **在 `setup()` 里设 `PATH`，让所有 Command 天然继承。** 否决：`process::Command` 在 `spawn` 时读父进程 PATH，不在 `Command::new` 时读，而 Windows 上父进程 PATH 还是只有系统 PATH。直接改 `std::env` 在 Tauri 多线程运行时里也是 `unsafe`（`env.rs` 里 `OnceLock` 模式的存在就是为了避开这点）。
- **新增一个注册表无关的 fallback，让用户能在 settings 里手动指 `git.exe`。** 暂缓：能镜像现有的 `pnpm_path` / `npm_path` 槽位，但每多一个手动配置槽都意味着用户体验更差——「按你用户 PATH 来」才符合每个 GUI shell 的预期。用户第一次撞到的失败应该是个能修的 PATH，而不是改 settings.json。