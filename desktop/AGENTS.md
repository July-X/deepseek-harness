# AGENTS.md — dsh-desktop

桌面壳的 agent 约定。模块布局与数据流见 [docs/architecture.md](docs/architecture.md)；用户文档见 [README.md](README.md)。

## 范围

- **不侵入内核代码**：`packages/`（`@deepseek-ai/dsh` 各分组）是用户从 npm registry 自行安装的源码，桌面壳**不**打包、不重发布、也不修改——改动不会随 release 抵达用户机器。需要影响工作台窗口或内核行为时只走自有边界（`src-tauri/` Rust 进程 + `ui/` 管理面板）：脚本/样式覆盖用 `WebviewWindowBuilder::initialization_script()`、内核生命周期在 `kernel.rs`、配置落盘在 `settings.rs`。"只能改内核"的改动推到对应内核 PR。完整边界依据见仓库根 [AGENTS.md](../AGENTS.md)「dsh-desktop 范围约束」。
- **独立交付物**：不加入仓库根 pnpm workspace，不参与上游 lint / hygiene / release 门禁。`desktop/pnpm-workspace.yaml`（allowBuilds 放行 esbuild）让 pnpm 把本目录当独立根——直接 `pnpm install` 即可，**不要**加 `--ignore-workspace`（它会让 pnpm 跳过该文件，esbuild postinstall 在 strictDepBuilds 下变成硬错误）；缺 pnpm 时 `scripts/install.mjs` 回退 npm。
- **信任边界**：仅信任官方 `deepseek-ai` 仓库与 npm `@deepseek-ai` 命名空间；版本列表优先 npm registry，GitHub Releases 仅作回退。

## 命令

```sh
npm run deps                      # 安装依赖（pnpm 优先，缺失回退 npm）
npm run dev                       # tauri dev（自动先起 vite dev server，5173 热更新）
npm run dev:ui                    # 只起管理面板 dev server（纯前端迭代，浏览器里无 Tauri 桥）
npm run build                     # 本机构建（.dmg / NSIS；自动先 vite build → ui/dist）
npm run build:ui                  # 只构建管理面板 → ui/dist
cargo check                       # 在 src-tauri/ 内：快速编译检查
cargo clippy --all-targets        # lint，零警告基线
cargo fmt                         # rustfmt 格式化
```

UI 是 Vue 3 + Element Plus 单页应用（源码 `ui/src/`，Vite 构建到 `ui/dist/`，即 tauri.conf.json 的 `frontendDist`）。状态与动作集中在 `ui/src/store.js` / `plugins.js` / `skills.js` / `progress.js` / `logs.js`，组件只读状态、调动作；与 Rust 的通信只允许走 `ui/src/bridge.js` 的 invoke/Channel。触发 IO 的按钮必须挂 loading（`loading.js` 的 `withLoading(key, …)` + `:loading="isLoading(key)"`；长任务走 `progress.js` 的 `withProgress`）。验证：改完跑 `npm run build:ui`；Rust 改动至少 `cargo check`；提交前 `cargo clippy --all-targets && cargo fmt`。

## 数据目录

`kernel::data_dir` 统一解析 `<dsh_home>/desktop/`（release）或 `<dsh_home>/desktop-dev/`（debug）。`lib.rs` 的 `setup()` 必须通过它取目录——不要绕回 `app_data_dir()`。debug 端口 3091，release 端口 3090（`kernel::DEFAULT_PORT`）；用户保存过的 port 优先于 `Settings::default()`。优先级与错位原因见 [docs/architecture.md](docs/architecture.md)。

## 插件实现

`plugins.rs` 的 pnpm 标志（`--ignore-workspace --config.node-linker=hoisted --reporter=append-only`）、`.npmrc` 关闭 `minimumReleaseAge`、装前删旧 lockfile、`materialize_one` 经 `read_link` 修复双层 symlink 等实现规则见 [docs/plugin-internals.md](docs/plugin-internals.md)；设计层（用户可见的目录布局、双模式、接线、目录浏览）见 [docs/plugin-management.md](docs/plugin-management.md)。

## 约定

- 用户可见文案用简体中文；错误信息必须包含可操作的下一步与相关日志路径。
- 工作台 UX：内核是实现细节，概览页只暴露「启动工作台 / 关闭工作台」单按钮状态机（启动 = 拉起内核 + 轮询等待就绪 + 自动打开窗口；失败自动弹出内核日志）；「打开工作台窗口」「查看日志」是次级 ghost 入口。不要把内核生命周期拆成独立按钮。
- 长任务失败时进度面板保持开放，由用户手动点「关闭」收起——不要在 catch 后自动隐藏。
- 日志双轨：UI 只显示有限行数的实时流（ANSI 在前端剥离），完整原始输出始终落盘 `<data_dir>/logs/install-<version>.log` 与 `kernel.log`；报错信息引用日志路径。
- 图标：全仓库统一双母版（`assets/whale-icon.svg` + `assets/whale-icon-small.svg`），改设计只改这两个 SVG，再跑 `scripts/build-icons.sh` 一次性再生成全套。眼睛射线必须用 `<polygon>`（不是 `<path>`）。bundle 瓦片、RGBA、再生成规则与增量构建触发见 [docs/icon-design.md](docs/icon-design.md)。
- Windows 兼容：`.cmd` 脚本 spawn 须经 `%ComSpec% /C`（长任务见 `process::run_with_progress` / `run_pnpm`，一次性收集输出见 `process::script_output`）；GUI 子进程必须过 `process.rs` 的 `quiet()`（CREATE_NO_WINDOW）；检测 PATH 上的工具一律扫 `env::merged_path()`（GUI 只继承系统 PATH，用户 PATH 里的 `%AppData%\npm` 等条目要合并注册表 `HKCU\Environment\Path` 才可见）；短生命周期工具（`git ls-remote`、`tar -xzf` 等）走 `process::command_with_path(program)`。
- 频繁轮询的路径不要拉起子进程：`get_status` 每 2.5s 一次，`node::resolve` 结果按 `node_path` 缓存进 `AppState.node_cache`；新增轮询字段先确认是纯文件/网络读取。
- Tauri 同步命令跑在主线程：凡涉及进程 spawn、网络请求或目录树的命令（启动/停止内核、装删版本、拉 releases、插件操作）必须 `async` + `tauri::async_runtime::spawn_blocking`（模板见 `run_plugin_command`）；闭包里用 `AppHandle` 重新 `app.state::<AppState>()`，不要 move `State`。
- 进程回收：内核子进程在 Unix 上 `setsid` 独立进程组，停止时杀整组；应用退出时兜底回收（`lib.rs` 的 `RunEvent::Exit`）。新增后台进程沿用该模式。
- 版本发布由 `.github/workflows/desktop-release.yml` 负责（tag `desktop-v<version>` 触发，且只接受 develop 上的 commit——tag 指向其他分支或 dispatch 选其他 ref 都会在 verify 步失败）；发版前同步 `package.json` 与 `tauri.conf.json` 的 `version`。workflow 用 `TAURI_SIGNING_PRIVATE_KEY` secret 给更新制品签名，与 `tauri.conf.json` 钉死的 updater pubkey 配对——轮换密钥时两者必须一起换。**`releaseDraft` 与 `prerelease` 字段都 hardcode 为 `false`**。
- 外壳自更新走 `tauri-plugin-updater`（见 `src/updater.rs`）：endpoint 解析 `/releases/latest/download/latest.json`，所以发布版本必须不是 draft、不是 prerelease，否则 updater 拿到 404 时 `updater::check()` 把 `ReleaseNotFound` 当 empty state 处理（UI 显示"已是最新"），看起来像是"没有更新"，实际是 endpoint 路径错了。

## 工具调用与沙箱

DSH 把工具调用与文件策略解耦：`runtime-context` 的「Current DSH file policy」给出当前会话的 `sandbox_permissions`（`workspace-write` / `danger-full-access`），agent 必须据此调用 `fs.edit / fs.write / pwsh` 等工具。误传 `sandbox_permissions` 是当前唯一常见的「沙箱升级失败」根因。

**禁止同模式重声明 sandbox_permissions。** `fs.edit / fs.write / pwsh` 等工具调用里若带 `sandbox_permissions` 字段，其值必须**严格大于** `runtime-context` 给出的当前模式；与当前模式相同或更窄都会触发 `sandbox escalation to "<mode>" is not strictly wider than this call's current "<mode>" mode` 错误。出现频率最高的触发场景：

- 当前模式已是 `danger-full-access`，但工具调用仍显式写 `sandbox_permissions: "danger-full-access"` —— 这种「重声明」会被工具拒绝。
- 当前模式是 `workspace-write`，却传了 `sandbox_permissions: "workspace-write"` —— 同样被拒。

**正确做法：**

1. 默认情况下，`fs.edit / fs.write / pwsh` 调用**不要**传 `sandbox_permissions` 字段；当前 `runtime-context` 的模式就是工具的实际权限，无需重复声明。
2. 只有当调用需要的模式严格大于 `runtime-context` 当前模式（例如当前是 `workspace-write`，但本步要写工作区之外的文件）时，才显式传更高级别，并附带一句非空 `justification`（即 `sandbox_permissions` 升级理由），由当前会话的审批策略决定是否放行。
3. 在 `runtime-context` 标注「Approval prompts are disabled」的会话里，**禁止**任何 `sandbox_permissions` 重声明（含更高级别）；只有完全省略字段的调用才会通过。原因是沙箱把这种显式重声明视为升级请求，而禁用审批的会话无法放行任何升级请求。

**工具实现层的硬性检查（agent 自己实现工具时也要遵守）：** `fs.edit / fs.write / pwsh` 等工具在解析完请求后，应自行吞掉同级别 `sandbox_permissions`：

- 若传入的 `sandbox_permissions` 与当前模式相同，silent pass（视作未传），不要向调用方报错。
- 只有当传入的 `sandbox_permissions` 严格大于当前模式，才走升级路径并校验 `justification` 非空。
- `justification` 缺失或仅含空白字符时，必须报错并说明「同/更高级别 sandbox_permissions 重声明时省略字段；更高级别必须附一句非空 justification」。

这样同级别重声明永远是 silent pass，会话里就不会再出现那条错误。

## 已知坑

速查表见 [docs/troubleshooting.md](docs/troubleshooting.md)。