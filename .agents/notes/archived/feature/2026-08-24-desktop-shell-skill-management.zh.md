# Agent Note: 桌面端管理社区技能

Status: implemented
Archived: 2026-08-28

[English](2026-08-24-desktop-shell-skill-management.md) | 中文

## 问题

桌面端通过中央库、按内核物化、profile 接线管理社区**插件**，但社区**技能**没有同等路径。技能是 `dsh-skill-filesystem` 从 `DSH_HOME` 等固定根扫描的指令数据（`SKILL.md` 或带 frontmatter 的平 Markdown）；没有壳管理的入口，用户只能手动把仓库 clone 到正确目录，再重启工作台让 watcher 重新发现。

## 决策

**技能沿用插件的中心库模式，但把所有接线步骤折叠掉。** `~/.dsh/skills-store/` 存权威副本与 `.dsh-source.json` 溯源；`<DSH_HOME>/skills/` 是内核读取的 user-dsh 根，也是每个已安装内核共用的**单一**物化目标。因为 `dsh-skill-filesystem` 已经扫描该根并用 chokidar 监视，物化一个技能即接线完成——不写 cordis、不跑 pnpm、不按内核复制。

**安装单位=包，物化单位=技能。** npm tarball / git 仓库 / 本地文件夹里可能含多个技能（monorepo 布局 `skills/<name>/SKILL.md`）；壳按深度 ≤3 扫描，逐技能建链接。启停切换即建/拆单条链接。更新重新扫描上游、重链路径变化或 copy 模式下的技能、摘除上游移除的技能，并保留每个幸存技能之前的启用状态。本地文件夹不进版本检查，但保留手动「重新同步」走相同的增删逻辑。

**热生效取代重启。** 内核的 chokidar 监视加上 `skills/change` 失效事件，让每一次安装/卸载/启用/停用都对运行中的会话在下一次模型步骤前可见，无须重启内核——这与插件（profile 层在启动时快照）正好相反。

**frontmatter 由壳按顶层子集在安装前预校验。** 解析器只读顶层 `name` / `description`（含去引号）；内核会静默忽略的候选以警告形式出现在安装进度中，让用户看到「上游不会读它」而不是「装了却不出现」。整包零个可用技能时安装失败并给出原因。

**暂存与崩溃恢复沿用插件词汇。** `.tmp-<pid>-<nanos>` → `.new-<…>` → `.backup-<…>` 配 `.dsh-id` 标记，启动 `reconcile()` 按 id 分组恢复。`.dsh-id` 标记必须在 fetch 之后写入：`git clone` 要求目标目录为空，`npm` tarball 解包会覆盖标记，`copy_tree` 又先 `remove_dir_all` 再复制——预 fetch 写入只在 npm 路径下「意外能跑通」。同时该函数为启用技能补断链、清退停用技能的残留条目、清扫指向中央库但不在当前清单的活动根 symlink；普通文件、目录、指向 `skills-store/` 以外的链接都是用户内容，绝不动。

**v1 只出手动安装，不做社区目录卡。** 技能面板与插件面板同样只在手动安装行收纳 `<input>` + 物化模式 `<select>` + 安装按钮，解析同一组地址形态（npm spec、带 `#tag` 的 git URL、`owner/repo` 简写、`local:` 前缀 / 绝对路径 / `~/…` / Windows 盘符路径的本地文件夹）。GitHub `dsh-skill` topic 作为页脚链接常驻，用户在那边浏览社区资源后把地址粘贴回手动安装行。社区目录卡需要稳定的技能 hub feed，目前尚未部署——设计文档里保留 URL 常量与 hub/市场 JSON 解析形状，将来重新启用时只需在 `ui/skills.js` + `commands.rs` + `skills.rs` 做局部增量。

## 备选方案

**为每个技能单独物化到 cordis 管理的路径或 `Config.customSkillDirs` 入口。** 否决：需要写一个 cordis patch 层，而壳明确不写这块（`cordis.patch.yml` 由用户持有），并且每次改动都会引发启动重载。内核的 user-dsh 根已经是现成且被监视的接线点。

**按内核物化（每个内核版本各自 `plugins/<id>`-式链接）以与插件对齐。** 否决：技能不依赖内核的 `node_modules`，按内核复制买不到任何东西，反而让每条壳命令都得遍历 `kernel::list_installed`——切换内核要做无用功。

**在 `~/.dsh/skills-store/` 之外再开 `~/.dsh/skills/` 给非壳管理内容。** 跳过：内核 user-dsh 根和壳物化目标本来就重叠在 `<DSH_HOME>/skills`。共用目录让内核视图单源；壳的所有权记在 `store.json`，孤儿清扫通过「链接是否指向 `skills-store/`」识别，保护用户手放内容。

**通过新依赖做完整 YAML 解析。** 否决：内核加载时已经校验全文，壳只要在能预览到 name/description 时响亮失败就够。顶层子集解析器覆盖所有常见 frontmatter，又不增加依赖。

**在中央库里自动跑 `pnpm install` 支撑带依赖的技能。** 否决：技能是指令数据而非带运行期依赖的包；少数引用脚本的通过 `resourceBase` 在加载时解析。若未来某技能真要传递依赖，正确做法是单独注册 agent provider，而不是在壳里给每个技能配构建管线。

## 后果

- `skills::SkillStoreItem.skills[].path` 是不透明的——指向包相对路径下的 bundle 目录或平文件。上游内部布局改名不影响用户可见的 frontmatter 名，但直接遍历中央库的下游工具必须尊重这个字段。
- 活动根条目名是 kebab-case 的 frontmatter `name`，不是 bundle 目录名。两个包都发布名为 `pdf` 的技能，只有上游项目自己解决冲突——壳拒绝单包内的 frontmatter 重复，但跨包冲突由先安装者占住链接。
- 桌面壳在 `error.rs` 多了一个 `Skill` 错误变体（与 `Plugin` 平级），用户看到的是「技能错误：…」而不是笼统的「I/O 错误」。
- 社区目录卡在 v1 明确不做（没有稳定的 hub feed），设计文档预留了 URL 常量与 JSON 形状，将来重新启用时只在 `ui/skills.js` + `commands.rs` + `skills.rs` 做局部增量，不是重新设计。
- 用户文档 `desktop/README.md` 加了一条与插件管理平行的「技能管理」要项；更细的设计在 `desktop/docs/skill-management.md`，与本次改动同步更新以贴合已交付事实。