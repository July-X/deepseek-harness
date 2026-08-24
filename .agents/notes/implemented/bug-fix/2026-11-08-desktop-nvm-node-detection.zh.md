# Agent Note: Desktop shell discovers nvm-managed Node installs

Status: implemented

[English](2026-11-08-desktop-nvm-node-detection.md) | 中文

## Problem

桌面壳通过 `desktop/src-tauri/src/node.rs::resolve` 定位运行 `dsh web` 的 Node 运行时。修复前的搜索顺序——显式 `settings.node_path`、PATH 扫描、固定的几组系统常见位置（`/usr/local/bin/node`、`/opt/homebrew/bin/node`、`/usr/bin/node`、`C:\Program Files\nodejs\node.exe`、`%LOCALAPPDATA%\Programs\nodejs\node.exe`）——能覆盖直接安装和 Homebrew，但漏掉了所有由 [nvm](https://github.com/nvm-sh/nvm)（macOS/Linux： `$NVM_DIR/versions/node/vX.Y.Z/bin/node`）与 [nvm-windows](https://github.com/coreybutler/nvm-windows)（`%NVM_HOME%/vX.Y.Z/node.exe`，通过 `%NVM_SYMLINK%` 暴露）管理的 Node。

GUI 进程在 macOS 上只继承 launchd 的 PATH（`/usr/bin:/bin:/usr/sbin:/sbin`），在 Windows 上只继承 Window Station 的系统 PATH；`npm install -g`、Homebrew、nvm 添加的位置都在 `HKCU\Environment\Path` 与用户的 shell 启动文件里，不在 GUI 子系统进程在 create-process 时继承到的环境里。内核安装路径因此报出 `未检测到 Node.js。请安装 Node.js 22.19+（或 >=24）后重试，或在设置中手动指定 node 路径。`，唯一退路是手动路径槽位——可行，但多数 nvm 用户根本走不到这一步，因为报错文案没有提到 nvm。

找到 Node 之后还有第二类较小的失败：`kernel::install_version` 用 `extra_path_dirs = [pnpm_exe.parent()]` 调用 pnpm。当用户在 `settings.pnpm_path` 里固化了 pnpm，而解析出的 `node` 在别处时，pnpm 的 `#!/usr/bin/env node` shebang（以及任何会调用 `node` 的 lifecycle 脚本）会报 `env: node: No such file or directory`，尽管父进程本来就能拉起 pnpm。

## Decision

Node 自动检测现在直接扫描 nvm 管理的布局。PATH 扫描仍然先跑（终端启动的 dev shell 在 `nvm use` 之后会把正确版本落在这里），接着是 nvm 管理的安装，最后是常见的系统位置。

macOS/Linux（`nvm-sh`）下，壳从 `$NVM_DIR` 或 `$HOME/.nvm` 解析 nvm 根目录，枚举 `<root>/versions/node/<vX.Y.Z>/bin/node` 目录，并解析 `alias/default` 文件——最多跟随 5 跳，手手-edited 的环不会让流程卡死。与解析后的 spec 匹配的安装项排在最前（精确匹配优先，然后是字符串以 spec 开头的最新已安装版本——对齐 nvm 对裸主版本别名如 `22` 的解释）。其余引擎兼容的安装项按从新到旧排序，引擎不兼容的安装项在探测前就过滤掉，因此每一项最多花一次子进程拉起的成本。

Windows（`nvm-windows`）下，激活态的连接点（`%NVM_SYMLINK%`，即 `nvm use` 选中的那个）排首位，接着是 `%NVM_HOME%` 与默认安装位置 `%APPDATA%\nvm` 下所有引擎兼容的 `<NVM_HOME>/vX.Y.Z/node.exe`，按从新到旧排序。nvm-windows 用连接点而不是别名文件来记录选择，所以这里没有默认 spec 要解析。

空结果消息同时点名了 engines 区间与 nvm 专属的安装路径（`nvm install 24 && nvm alias default 24`），并在探测过程中把最近的失败暴露出来（一个能跑但版本太旧的 Node），让用户分得清「这里没有 Node」和「你的 Node 太老了」。

`kernel::install_version` 接受 `node_dir: &Path` 参数，并把它放在 `pnpm_exe.parent()` 之前拼到子进程的 PATH 上。安装期间拉起的任何子 shell（pnpm 自身、会解析 `node` 的 lifecycle 脚本）看到的 `node` 与父进程一致，跟 pnpm 来自哪里无关。

`detect_node` 改为 `async`，并把 `node::resolve` 放进 `tauri::async_runtime::spawn_blocking` 里跑。检测会为每个环境候选（PATH + nvm 安装 + 系统位置）最多拉起一个子进程直到找到可用的 Node；把进程拉起从 Tauri 主线程移走，符合桌面壳里涉及文件系统与子进程的命令的约定。

检测结果被拆成两条不同的失败消息。当磁盘上没有任何候选——全新机器，没装 nvm，也没装系统 Node——`NO_NODE_FOUND_GUIDANCE` 列出三条独立的安装路径（版本管理器 nvm / fnm / volta；平台包管理器 brew / NodeSource + apt / winget；以及官方安装包 [https://nodejs.org/](https://nodejs.org/)），并附上「设置」里的手动路径槽位作为兜底。当一个候选能跑但版本不满足 engines 区间时，`NODE_TOO_OLD_GUIDANCE` 只列出升级命令。两条消息互不重叠：全新安装命令不出现在升级消息里，升级命令也不出现在无 Node 消息里，因为每个用户自己清楚是「装 Node」还是「升级 Node」。`ensure_pnpm` 在 `pnpm` 和 `npm` 都不可达时（这通常意味着用户磁盘上既没有 pnpm 也没有 npm）共用同一条无 Node 引导，因此无论哪个命令先触发失败，管理面板与安装进度面板都会给出一致的安装选项。

## Alternatives considered

**在 Windows 上读取 `HKCU\Environment` 中的 NVM_HOME / NVM_SYMLINK。** 因范围被拒。GUI 进程完全没有这两个变量时，注册表才是权威来源；但在已经存在 `desktop/src-tauri/src/env.rs` 合并逻辑的情况下加一处注册表读取是重复工作，而 Explorer 启动的子进程本就继承了用户环境变量，这件事很少被实际触发。默认 `%APPDATA%\nvm` 路径覆盖标准安装；非默认 `NVM_HOME` 的用户通常能看到同一个启动本壳的 Explorer 传递下来的环境变量，因此当前的扫描已经能命中。只有只有出现「环境变量真的缺」的具体失败信号时才考虑重新打开这条路径。

**通过 `bash -lc 'nvm which current'` 直接问 nvm。** 被拒。壳不能假设用户在 Windows 上有 POSIX shell 可用；命令要先 source `nvm.sh`；输出是脚本路径而不是二进制路径。直接扫盘跨平台、不依赖 shell 引号、用户在同一天装好 nvm 后无需重启 shell 就能工作。

**只用 `node_path` 作为 `cached_node` 的缓存键（当前实现）。** 因改动被拒。检测结果的确依赖机器状态（新装的 nvm 立刻出现在磁盘上），但状态轮询本身每 2.5 秒跑一次阻塞 worker，按 `node_path` 值单次命中是文档化的取舍。给缓存键加上文件系统监听器超出范围；用户点「检测 Node」或改一下设置就能让它失效。

**从 launch agent 那一层 source `~/.nvm/nvm.sh`，让每个 GUI 子进程都能看到 nvm 在 PATH 上。** 被拒。macOS 与 Windows 的 launchd / Explorer 在创建进程时已经提供一个稳定的环境；在这个基础上叠一层 shell-source 是脆弱的（文件移动、登录 shell 差异），并且抵消了直接扫盘带来的「不依赖 shell」契约。

**把 PATH 上的任何 `node` 当作权威。** 被拒。修复前的 PATH 扫描正是这一行为，并在系统 PATH 恰好包含较旧 Node（Homebrew 默认 `node@18`、残留的 `/usr/bin/node`）时给出假阳性检测结果。在回退之前先用 `compatible()` 过滤不兼容的版本，并在文案里把最近的失败暴露出来，让新路径保持诚实。

## Consequences

nvm 用户——以前必须去手动路径设置绕一圈——现在可以在管理面板里直接安装与启动内核，macOS、Linux、Windows 都覆盖。用户执行 `nvm install 24` 并 `nvm alias default 24` 之后，下一次状态刷新与安装按钮的诊断都能反映该版本。多版本 nvm 且没有 `default` 别名的用户依然能拿到最新的引擎兼容安装。

探测为每个环境候选最多拉起一次子进程，直到找到可用的 Node。常见的单 Node 场景里成本就是一次 `node --version`，相对修复前的行为没有变化。当用户装了很多 nvm 版本时，遍历受 `compatible()` 对目录名的预过滤（不兼容的不会进入探测）限制，因此通常 2–5 个安装项的设置下墙钟成本依然很小。

即使用户在 `settings.pnpm_path` 固化了 portable pnpm 并运行 nvm 管理的 Node，安装路径也能可靠地解析 `node`，从而关掉 `pnpm add` 内部 `env: node: No such file or directory` 这一类失败。`node::resolve` 是唯一的真实来源；`commands::promise_pnpm` 继续把解析后的 Node 喂给它，`node_dir` 参数把这一信息传到内核安装里，而不需要 `commands::install_kernel` 复制解析逻辑。

空检测文案同时点名 engines 区间与 nvm 专属的安装路径。安装有 nvm 但尚未安装任何 Node 的用户看到可操作的指引，而不是一句要求他们离开本应用去装 Node 22.19+ 的泛泛之言。

既没有 nvm 也没有装 Node 的全新机器现在看到三条独立的安装路径，而不是只有一条：版本管理器（nvm / fnm / volta）、平台包管理器（macOS Homebrew、Debian/Ubuntu 的 NodeSource + apt、Windows winget）、以及官方安装包 [https://nodejs.org/](https://nodejs.org/)，并以「设置」里的手动路径槽位作为兜底。这种情况下探测结果没有 near-miss，因此消息告诉用户「装 Node」而不是「升级 Node」，跳过不适用的升级命令。`pnpm` 自动安装失败、且 `npm` 本身也不可达时（只有用户磁盘上既没 pnpm 也没 npm 才会走到这里），同样的无 Node 引导会被附上，因此管理面板与安装进度面板给出一致的安装选项。

## Testing

`cargo test --lib node::` 覆盖了纯函数：别名 spec 归一化、`format_version`、部分 spec → 最新匹配（对齐 nvm）、不兼容版本丢弃、未知 spec 退回到从新到旧扫描，以及 alias-chain 的有界跟随（解析到具体版本、alias 目标不是文件时的部分 spec、跳数上限控制的环、空别名文件）。`resolve_alias` 在 `std::env::temp_dir()` 下为每次测试构造独立的临时目录，并行测试运行之间不会撞车。

`cargo test --lib` 覆盖更广的表面（`process::merge_extra_path`、`settings`、`registry`、`version`、`env::parse_reg_path`、`env::merge_paths`）。Windows 上两个 `plugins::tests` 失败（`computes_relative_paths` 的路径分隔符不一致、`materialize_link_then_copy` 的 symlink 回退）是本分支上预先存在、与本改动无关的问题。

`cargo clippy --all-targets` 在新代码上是干净的（`kernel::reap_orphans` 那条不相关的告警在改动之前就已存在）。`cargo fmt --check` 在新代码上静默通过。