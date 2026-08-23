# Agent Note: 插件存储的崩溃安全交换与陈旧更新徽标抑制

Status: implemented

[English](2026-08-23-plugin-store-crash-safe-swap.md) | 中文

## 问题

`plugins::fetch_into_store` 把安装/更新的交换过程执行成连续三次 fs rename：

```text
rename(final_dir -> .old-*)
remove_dir_all(.old-*)
rename(tmp -> final_dir)
```

中间的 `remove_dir_all` 留下一段崩溃窗口：`final_dir` 已经不存在、旧内容已经被清掉、新内容还在 `tmp` 里。期间发生内核 panic、对 Tauri 进程 `kill -9`、或机器断电，就会留下一个没有任何 live plugin 的插件存储——`materialize_one` 之后无法为任何内核建立链接，下一次在 profile 里执行 `pnpm install` 会因为链接目标不存在而报错。外壳本身察觉不到；用户看到的是"插件不存在"或者下次内核启动时硬崩溃。

另一方面，`status()` 会对顶层 `updates` 计数走 `is_newer_than` 过滤，但逐行的 `latest_version` 字段会被原样透传到 UI。`desktop/ui/plugins.js` 是按字段是否为真来渲染"有更新"徽标的，所以一次成功的 `update()` 把 `latest_version` 与 `installed_version` 同步成同一字符串之后，行上仍然带着徽标和"更新"按钮。顶层的"N 个更新"药丸消失得对，这反而让行级幽灵看起来像渲染器回归。

## 决策

**三段式 staging 命名 + `.dsh-id` 标记。** `fetch_into_store` 现在在每个 staging 目录里写一个 `.dsh-id` 标记（目录名是 `tmp-<pid>-<ts>`、`new-<pid>-<ts>`、`backup-<pid>-<ts>` ——pid+ts 防止并发或"崩溃后再恢复"撞名），并按以下顺序跑交换：

```text
fetch_git/fetch_npm   -> tmp-<pid>-<ts>
validate_plugin(tmp)  --+
                        |  ok
rename(tmp -> new)
rename(final -> backup)
rename(new -> final)
remove_dir_all(backup)
```

任何步骤崩溃都会让 live plugin（`final_dir`）落到两种可恢复状态之一：要么指向旧版本（交换开始之前未被触碰），要么指向新版本（发布 rename 之前已经通过校验）。`final_dir` 短暂缺失的过渡窗口会在下次启动时被 `reconcile_store` 调和。

**`reconcile_store(data_dir)` 在 `lib::setup` 中无条件运行。** 恢复扫描读一次 `~/.dsh/plugins/`，按 `.dsh-id` 把 staging 目录分组，然后按插件 id 应用下表：

| `final_dir` | `.new-*` | `.backup-*` | `.tmp-*` | 动作 |
| --- | --- | --- | --- | --- |
| 存在 | 任意 | 任意 | 任意 | 移除所有 staging（发布后清理或过时尝试） |
| 缺失 | 无 | 有 | 无 | 回滚：rename `.backup-*` → `final_dir` |
| 缺失 | 有 | 无 | 无 | 发布：rename `.new-*` → `final_dir` |
| 缺失 | 有 | 有 | 任意 | 回滚（更稳；用户保留已知可用的旧版本） |
| 缺失 | 无 | 无 | 有 | 未完成的 fetch；移除 `.tmp-*` |

当多个 staging 共享同一个 id（恢复本身也崩溃了）时，按字典序最大的后缀（`pid-ts` 越靠后越新）取最新一份，其余移除。恢复时把任何缺 `.dsh-id` 标记的 staging 原样保留，所以老外壳写的不同 staging 名不会丢用户数据。

**`status()` 把逐行的 `latest_version` 字段也走一遍 `is_newer_than`。** Rust 一侧成为"什么算更新"的单一权威：当 `latest == installed` 时字段变成 `None`，顶层 `updates` 计数保持 0（它原本就过滤），UI 的 `if (row.latest_version)` 检查就不会再渲染幽灵徽标。

## 备选方案

**在 Linux 上使用 `renameat2(RENAME_EXCHANGE)` 实现原子交换。** 不予采用：macOS 没有它，平台相关的 fallback 仍然需要同一套恢复脚手架。staging + 恢复这套方案在所有 posix 系统以及将来的 Windows 端口上行为一致。

**恢复时永远把 `.new-*` 排在 `.backup-*` 前面。** 不予采用：旧版本才是用户实际加载到 profile 里跑过的；保留它意味着"崩溃-中-更新"的用户插件仍然可用，点一下重试就能拿到失败的更新。`.new-*` 的内容已通过校验，所以 promote 也安全，但回滚是更保守的默认。

**用 store 根目录的标记文件探测 staging 残留。** 不予采用：`setup()` 已经把 store 恢复作为第一次触碰到插件的步骤，而标记文件本身也要靠同样的扫描去找出哪些 staging 存在。一次带前缀匹配的 `read_dir` 比"写标记-再扫"便宜。

**让 UI 重新跑 `cmp_versions` 来判断行级徽标。** 不予采用：同一个比较逻辑会存在于三处（Rust `is_newer_than`、Rust `cmp_versions`、JS 移植）并会漂移。在 Rust 边界过滤意味着 wire 格式（`latest_version: Option<String>`）本身就编码了"较新"语义，UI 不可能因为忘记再去查一次就渲染陈旧数据。

**用纯 CSS 隐藏徽标。** 不予采用，理由同上：JS 检查是 `if (row.latest_version)`，加 `data-latest==installed` 属性和 CSS 选择器还是允许后续渲染器渲染错的内容。从源头修正数据就一处修好。

## 影响

- 中途崩溃的插件更新会让用户保留之前能用的插件（回滚），或者拿到刚通过校验的新插件（发布），永远不会是缺失插件。
- `reconcile_store` 每次外壳启动都无条件运行；正常路径就是一次 `read_dir` 没有活儿要干。
- `.tmp-*` 不再被复用作"已校验"阶段的 staging 名；rename 到 `.new-*` 才是向恢复扫描发出的"可以发布"信号。
- 插件更新失败（比如 pnpm build 因为别的原因仍然失败）保留原有错误路径：`fetch_git` 返回错误，`tmp` 被移除，`final_dir` 不被触碰。
- 逐行的 `latest_version` 字段在"已记录的远端版本不比已装版本新"时一律为 `None`，所以即便后续渲染器忽略顶层计数，UI 也不可能渲染出幽灵徽标。
- 紧接其前的那次提交 `fix(desktop): prepend pnpm's bin dir to child PATH for plugin builds` 是 `fetch_git` 的 `pnpm install --enable-pre-post-scripts` 步骤真的能拿到 `node` 跑 `prepare` 的前提。没有它，`fetch_git` 返回错误，`fetch_into_store` 过不去 Phase 1，staging 目录在 swap 开始之前就被移除。两个 commit 合起来关掉了用户报告的两半安装时失败模式。
