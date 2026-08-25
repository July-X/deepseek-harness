# Agent Note: Remove Windows directory symlinks with RemoveDirectory

Status: implemented

[English](2026-08-25-windows-dir-symlink-removal.md) | 中文

## Problem

桌面壳把中央库里的插件与技能以目录符号链接的形式物化到内核和技能根目录。在 Windows 上重新物化（更新、同步、切换模式）会以 `复制插件到内核失败：系统找不到指定的文件 (os error 2)` 失败，并留下悬空链接、孤儿中央库目录和无法清退的 profile manifest。故障链是：`DeleteFile` 拒绝删除目录符号链接（`ERROR_ACCESS_DENIED`）——只有 `RemoveDirectory` 能删——被吞掉的 `let _ = fs::remove_file(..)` 让链接原样保留；随后的建链因 `ERROR_ALREADY_EXISTS` 失败并降级为复制；复制又顺着残留的链接把中央库目录复制到自己身上。单个插件的失败还会中止整轮接线，卸载残留因此永远得不到清退，store warning 永久残留。skills 模块存在同样的"对目录符号链接调 `remove_file`"模式。

## Decision

`desktop/src-tauri/src/plugins.rs` 与 `skills.rs` 各自拥有一个 `remove_link` 助手：先尝试 `fs::remove_file`，失败回退 `fs::remove_dir`，覆盖各平台上的文件与目录链接（unix 的 `unlink` 两者通吃，回退实际只在 Windows 触发）。围绕这一根因，插件物化路径在同一次改动中整体加固：`copy_tree` 的每个 IO 错误都带具体失败路径，遇悬空链接跳过而非中止，并通过比对递归栈上各目录的 canonical 路径检测链接环（macOS/Linux 上 `node_modules` 全是符号链接，pnpm 循环依赖会形成环）——菱形共享不是环，照常通过。接线不再因单个插件失败而中止：失败插件聚合成一条错误，健康插件照常接线、卸载残留照常清退。新增清扫会移除中央库已不持有的内核插件条目——前提是能证明归外壳所有（有 meta 记录）或已损坏（链接悬空）。`uninstall` 先删中央库目录，锁失败时给出"关闭工作台后重试"的明确报错，不再留下孤儿目录。`reconcile_store` 清理没有 id 标记的暂存目录（包括旧版 `.tmp-` 命名），同时放过名字带暂存前缀的正式插件。`skills.rs` 另外识别 `\\?\` verbatim 路径，使本地技能包的「更新」在 Windows 上可用。

## Alternatives considered

**按平台 cfg 分别删除（Windows 用 `remove_dir`，其他用 `remove_file`）。** 不予采纳：调用点并不知道链接指向文件还是目录，而"两种都试"是一个无 cfg 面的可移植助手。

**用递归深度上限代替 canonical 环检测。** 不予采纳：Windows 上超过 `MAX_PATH` 的路径会让 `metadata` 报出与真实悬空链接相同的 `NotFound`，环在触达任何上限之前就被当作"悬空"静默跳过；canonical 祖先比对在各平台上都是在递归第二层就捕获环。

**沿用旧行为，首个插件失败即中止接线。** 不予采纳：失败发生时该插件的物化已被破坏，而中止还会挡住清退卸载残留的 manifest 修剪——正是本次事故中故障不断累积的原因。

## Consequences

Windows 上的更新/同步/切换模式不再破坏插件与技能的物化，单个损坏插件降级为一条指名警告而非卡死整轮接线。`copy_tree` 为每个目录多付一次 `canonicalize`。链接删除规则与 [Unlink fixture junctions before recursive deletion](2026-08-12-unlink-fixture-junctions-before-delete.zh.md) 的 junction fixture 规则互补：那篇负责"递归删除会跟随链接进入目标"，本篇负责"如何删除链接本身"。单元测试在 unix 与 Windows 双平台钉住：链接后复制的再物化、悬空链接跳过、环中止、孤儿清扫、无标记暂存清理、带前缀名的正式插件保护，以及单个插件失败时接线整体存活。
