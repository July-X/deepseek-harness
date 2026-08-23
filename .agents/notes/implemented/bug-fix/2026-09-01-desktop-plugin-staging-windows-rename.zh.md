# Agent Note：插件 staging 目录的 swap 在 Windows 上能扛住非空目标，并同步清掉临时目录

状态：已实现

[English](2026-09-01-desktop-plugin-staging-windows-rename.md) | 中文

## 问题

前面几轮修复（PATH 继承、tar `-C` 目录、fetch_npm 目标）让安装跑到 `fetch_into_store` 之后，又在 `fs::rename` 这步撞了 `I/O 错误：目录不是空的。 (os error 145)`。两个互相纠缠的 bug：

**`new_staging_dir` 在 Windows 上既冗余又不安全。** 函数在创建 staging 目录后立刻往里写 `.dsh-id`，然后 swap 时把装着内容的 `tmp` 重命名到一个只有 `.dsh-id` 一个文件的 `new` 上。在 Unix 上 `rename(2)` 能原子地替换目录，所以这样工作。在 Windows 上 `MoveFileEx` 在目标目录非空时会用 `ERROR_DIR_NOT_EMPTY`（os error 145）拒绝，rename 失败，用户看到的就是一行没头没尾的 `I/O 错误`。任何第二次走 swap 都会撞同一道墙——上一次失败留下的 `.new-*` 目录里还留着 marker，新的 helper 里 `let _ = fs::remove_dir_all(&dir);` 把清理错误吞掉，再 `fs::create_dir_all(&dir)?` 在已存在的非空目录上返回 `Ok(())`，于是 caller 拿到一个"全新"的目录，里面却塞着上次残留的内容。

**发布后的 backup 清理是 best-effort 且静默的。** swap 后面的 `let _ = fs::remove_dir_all(&backup);` 让一次失败的清理静悄悄留下一个 `.backup-*` 目录、不报错。叠加上面的 rename 失败，用户的 store 里会越攒越多脏 staging 目录，每个都成为下一次 `ERROR_DIR_NOT_EMPTY` 的火种。

bug 从最初的社区插件 commit（7ebc6f5a352，2026-08-22）就埋着——但跟之前的修复一样，Windows 是唯一能看到的平台。macOS 和 Linux 的 `rename(2)` 会对非空目录乖乖替换，所以同一段代码在那边"靠运气在工作"。这次用户撞上，是因为前面的修复终于把安装送到了 swap 这步，把 Windows 上一直会暴露的失败模式摆到台面上。

## 决策

**`new_staging_dir` 不再写 marker；它返回一个空目录。** marker 拆成单独的 `stamp_id_marker(dir, id)`。调用方负责在 swap 的**源**上盖 marker（`fetch_into_store` 里的 `tmp`）；rename 的目标（`new`，以及隐式的 `backup` 目标）保持为空，直到 rename 成功——之后 marker 要么从 `tmp` 一同过来（`new` 路径），要么显式盖（`backup` 路径，因为 `final_dir` 自己不带 marker）。在 Windows 上 rename 永远落在空目录上，`MoveFileEx` 接受。

**时间戳从秒升到纳秒**，同一秒里两次 `new_staging_dir`（崩了之后立刻重试就会撞到这个）不会再 `tmp-<pid>-<ts>` 撞名。清理现在只是兜底，不是承重墙：正常运行中撞名统计上不可能，清理卡住还是会通过 helper 的返回值冒上来。

**`new_staging_dir` 把 `remove_dir_all` 的错误往上抛**，不再吞掉。除了 `NotFound` 之外一律以 `AppError::Io` 返回。失败不再被"是的我已经清干净了"的 `Ok(())` 掩盖，而那个 `Ok(())` 其实根本没清。

**`fetch_into_store` 的 swap 在 rename 失败时前滚或回滚**，并把情况告诉用户：

- `tmp → new` 失败：验过的内容留在 `tmp` 磁盘上；用户看到 `将暂存目录提升到 .new-* 失败：<detail>`。下次重试或 `reconcile_store` 启动时能接力推广。
- `final_dir → backup` 失败：新内容用把 `new` 提升到 `final_dir` 的方式前推（让这次安装实际生效），用户看到 `插件已发布，但备份旧版本失败（<detail>）；下次更新若失败将无法回滚`。部分成功的警告是响的。
- `new → final_dir` 在 backup 已经成功之后失败：把上一个活的插件从 `backup` 还原回 `final_dir`，用户看到 `发布新版本失败，已回滚到旧版本：<detail>`。`new` 变成悬空的 `.new-*`，下次启动 `reconcile_store` 来收。
- `new → final_dir` 失败且回滚也失败：用户看到 `发布新版本失败且回滚旧版本失败：<detail>`，`reconcile_store` 是下次启动时的恢复路径。

**发布后 backup 的清理改成同步并把失败冒上来。** `fs::remove_dir_all(&backup)` 失败时返回带用户可见消息的错误，提到 `reconcile_store` 是下次启动的修复路径。`.backup-*` 脏目录不再悄悄积压。

**`write_source_marker` 在 swap 成功后、清理后再调。** marker 文件（`.dsh-source.json`）只在 rename 成功、临时目录都清完之后才写到 `final_dir`。这样在任何错误路径上，`final_dir` 要么还不存在（没有插件），要么还带着上一个版本和它自己的 marker（通过 `reconcile_store` 恢复）。

## 后果

- npm 来源、git 来源插件的安装在 Windows swap 阶段不再撞 `ERROR_DIR_NOT_EMPTY`。
- `fetch_into_store` 里的每一次 rename 都有定义好的恢复路径，原本"rename 失败 → 沉默"的用户面没了。
- 发布后的 backup 在函数返回成功之前就清掉，用户不需要等下次启动 `reconcile_store` 来清理一次幸福的安装。
- `tmp → new` rename 失败时，验过的内容留在 `tmp` 而不是 `new` 里。`reconcile_store` 下次启动的规则已经覆盖"活插件存在 + tmp 孤儿"（丢弃 tmp）以及"活插件缺失 + tmp + …"（看哪些兄弟还在），所以这个状态是可恢复的。
- macOS/Linux 上的行为在幸福路径上没有变化。新的错误报告是跨平台的，只是把失败模式从"靠 `rename(2)` 魔法默默救回来"变成了每个平台都显式。

`plugins.rs` 里之前两个 Windows-only 测试失败（`computes_relative_paths` 和 `materialize_link_then_copy`）还是没动，依然留给它们自己的提交。

## 备选方案

- **把 swap 简化回"解到 `tmp`，然后直接把 `tmp` 重命名到 `final_dir`"。** 否决：三阶段 swap 是 crash-safety 的承重墙——没有 `backup` 的中途 rename 崩溃会让用户既没插件也没恢复点，`reconcile_store` 也是按三个名字建的。修复点在三阶段 swap 的实现里，不是它的形状。
- **用 `MoveFileEx` 加 `MOVEFILE_REPLACE_EXISTING` 在 Windows 上替换非空目录。** `std::fs::rename` 没暴露这个，要拉 `windows-sys` 或者一个薄的 FFI 包装。空目标设计避开整个兼容问题，每个平台都直接能用。
- **从 `fs::rename` 换成 copy-then-delete。** 否决：插件大小下 copy 很慢，copy 中途崩了会留下两份半截的拷贝，`reconcile_store` 区分不出来。空目标设计就多一个 helper 加一次 post-rename stamp，根本不 copy。
- **每个插件共用一个 staging 目录。** 否决：插件 id 才是身份，共用目录每次装都得重新校验。per-id 命名是让 `reconcile_store` 能把孤儿目录归到正确插件的依据。
- **`reconcile_store` 不只在启动时跑，每次安装开始也跑。** 否决，因为安装已经失败、我们要的是安装路径本身自愈。`reconcile_store` 是崩溃时的兜底，不是活失败的修复路径。新的错误报告让"未来想加 pre-install 清理"成为一个小增量。