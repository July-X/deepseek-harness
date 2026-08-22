# AGENTS.md — dsh-desktop

面向在本目录工作的 agent 的约定。背景与用户文档见 [README.md](README.md)。

## 定位与边界

- dsh-desktop 是 DeepSeek Harness 的 Tauri v2 桌面外壳：管理面板（`ui/` 静态前端）+ Rust shell（`src-tauri/`），负责内核版本管理和启动 `dsh web`。
- **独立交付物**：不加入仓库根的 pnpm workspace，不参与上游 lint / hygiene / release 门禁。在 `desktop/` 内安装依赖必须带 `--ignore-workspace`，否则 pnpm 会向上匹配仓库根的 `pnpm-workspace.yaml`，把整个 monorepo 装进当前命令。
- 信任边界：仅信任官方 `deepseek-ai` 仓库与 npm 的 `@deepseek-ai` 命名空间；版本列表来自 npm registry（GitHub Releases 仅作回退）。

## 命令

```sh
pnpm install --ignore-workspace   # 安装 @tauri-apps/cli（--ignore-workspace 必需，见上）
pnpm run dev                      # tauri dev
pnpm run build                    # 本机构建（.dmg / NSIS）
cargo check                       # 在 src-tauri/ 内：快速编译检查
cargo clippy --all-targets        # lint，零警告基线
cargo fmt                         # rustfmt 格式化
```

- UI 是零构建静态页面（`ui/index.html` + `app.js` + `styles.css`），无打包步骤；改完直接生效。JS 语法可用 `node --check ui/app.js` 快速验证。
- Rust 改动至少跑 `cargo check`；提交前跑 `cargo clippy --all-targets && cargo fmt`。

## 架构速览

```
ui/app.js + ui/plugins.js ──invoke(Channel)──▶ commands.rs ──▶ kernel.rs / plugins.rs ──▶ pnpm/git/tar 子进程
                                   │              │
                              settings.rs    releases.rs（npm registry → GitHub 回退）
                                   │
              ~/.dsh/desktop/{settings.json, kernels/, logs/, active.txt} + ~/.dsh/plugins/
```

- **commands.rs**：Tauri 命令层。长任务（内核安装）用 `spawn_blocking` + `tauri::ipc::Channel` 向 UI 推进度事件。同步命令里不要创建 webview 窗口（Windows 死锁），沿用 `open_harness` 的新线程模式。
- **kernel.rs**：内核生命周期。
  - 安装 = 在 `<data_dir>/kernels/<version>/` 写最小 stub `package.json` 后执行 `pnpm add --prefix … --ignore-workspace --config.node-linker=hoisted --reporter=append-only @deepseek-ai/dsh@<version>`。
  - `node-linker=hoisted` 保证 `node_modules` 扁平，内核入口固定为 `node_modules/@deepseek-ai/dsh/lib/bin.js`（`KERNEL_BIN_REL`）；改布局必须同步该常量与 `start()`。
  - `run_pnpm` 把 stdout/stderr 各用一个 drain 线程读入 mpsc channel，安装线程逐行回调 `on_progress` 并落盘日志——不要把两个管道放在同一线程顺序读取（会因管道缓冲区满而死锁）。
- **plugins.rs**：社区插件管理。详见下文「插件机制」一节。
- **releases.rs**：npm registry 全量版本 + dist-tags；registry 不可达时回退 GitHub Releases API 与 Atom feed。
- **node.rs**：Node 检测与 engines 校验（`^22.19 || >=24`）、pnpm 解析（显式配置 → node 同目录 → PATH）。
- **settings.rs**：`settings.json` 平铺结构（`node_path` / `pnpm_path` / `port`），serde default 兼容缺字段。

## 数据目录

外壳全部状态位于 `<dsh_home>/desktop/`（默认 `~/.dsh/desktop/`，`DSH_HOME` 可重定向），由 `kernel::data_dir` 统一解析并在启动时创建；`lib.rs` 的 `setup()` 必须通过它取目录——不要绕回 `app_data_dir()`。子结构：`kernels/<版本>/`、`logs/`、`settings.json`、`active.txt`。

## 插件机制

**布局**：

```
~/.dsh/
├── plugins/<id>/            # 中央 store：git clone 或 npm 提取
│   ├── package.json
│   ├── lib/                 # TS 插件的 build 产物（pnpm install 时 prepare 触发）
│   ├── node_modules/        # store 自身的依赖
│   ├── pnpm-lock.yaml
│   └── .npmrc               # 由 ensure_store_npmrc 写入（minimumReleaseAge=0）
├── profiles/<name>/         # dsh profile（声明 bundle 列表 + 插件 link 依赖）
│   ├── package.json         # {"dependencies":{"dsh-synapse":"link:../../desktop/..."}}
│   ├── node_modules/<id> → ../../../desktop/.../<id>
│   └── pnpm-workspace.yaml  # nodeLinker: hoisted, autoInstallPeers: false, minimumReleaseAge=0
└── desktop/kernels/<version>/plugins/<id> → ~/.dsh/plugins/<id>  # materialize_one 写入
                                               .meta/<id>.json      # mode + version + synced_at
```

**完整流程**（以 `install(spec)` 为例）：

1. **fetch**：git clone（深度 1）或 npm tarball 解压到 `~/.dsh/plugins/<id>/`
2. **ensure_store_npmrc**：写入 `~/.dsh/plugins/.npmrc`（`minimumReleaseAge=0`、固定 npm registry）—— pnpm v11 的 `minimumReleaseAgeExclude` 不支持通配符，必须直接关掉年龄检查
3. **install_store_deps**：`pnpm install --ignore-workspace --config.node-linker=hoisted --reporter=append-only`
   - 装依赖链 → 若有 `prepare` 脚本（`tsdown` / `tsc`）→ 触发构建 → `lib/` 就位
   - 装前**先删旧 `pnpm-lock.yaml`**：避开历史 lockfile 的 `minimumReleaseAge` 失效条目
4. **upsert_item**：写入 `~/.dsh/plugins/store.json`
5. **sync_kernels**：每个已装内核调用 `materialize_one`：
   - 解析 `resolved_source`（store 若本身是 symlink，展开到真实路径）
   - 校验现有 `target` symlink 是否等于 `resolved_source`——不等就重建（修复历史 double-symlink 链）
   - 调 `refresh_store_peers`：把内核 `node_modules/@deepseek-ai/*` 链接进 store 的 peer deps 解析路径
6. **ensure_wiring**：写 profile 的 `package.json` + `pnpm install` 把 `link:` 依赖铺到 `profiles/<name>/node_modules/`

**常见坑**：

| 现象 | 根因 | 修复 |
| --- | --- | --- |
| `minimumReleaseAgeExclude` 通配符不生效 | pnpm 不支持通配符，必须 `package@version` 全列 | 改用 `minimumReleaseAge=0` |
| `Cannot find module .../lib/index.js`（TS 插件） | `pnpm install` 没触发 `prepare` 构建 | 修好 `.npmrc` + 删除旧 lockfile 后重装 |
| 内核 `node_modules/<id>` 看着对但解析失败 | store 若本身是 symlink，套一层形成 double-symlink | `materialize_one` `read_link` store，链上一个跳转 |
| 重启后 `pnpm install` 报 `ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION` | 旧 lockfile 过期条目 | `install_store_deps` 先删 lockfile 再 re-resolve |

## 约定

- 用户可见文案用简体中文；错误信息必须包含可操作的下一步（如缺失依赖时给出安装指引）与相关日志路径。
- 工作台 UX：内核是实现细节，概览页只暴露「启动工作台 / 关闭工作台」单按钮状态机（启动 = 拉起内核 + 轮询等待就绪 + 自动打开窗口；失败自动弹出内核日志）；「打开工作台窗口」「查看日志」是次级 ghost 入口。新增用户可见操作时不要把内核生命周期拆成独立按钮。
- 长任务失败时进度面板保持打开并展示错误与日志区，用户手动点击「关闭」才收起——不要在 catch 后自动隐藏面板。
- 日志双轨：UI 只显示有限行数的实时流（ANSI 转义在前端剥离）；完整原始输出始终落盘 `<data_dir>/logs/install-<version>.log` 与 `kernel.log`。报错信息引用日志路径。
- 图标全仓库统一双母版：`assets/whale-icon.svg`（≥128px，完整红眼细节）与 `assets/whale-icon-small.svg`（≤64px、favicon 及 `ui/whale-icon.png`——面板顶栏只按 60 CSS px 显示；小尺寸下细节是亚像素，必须简化）。改设计只改这两个 SVG，然后跑 `scripts/build-icons.sh`（需 rsvg-convert + ImageMagick + macOS iconutil）一次性再生成：`src-tauri/icons` 全套（按尺寸选母版合成 ico/icns）、`assets/whale-icon-512.png`、`ui/whale-icon.png`（小母版渲染 128px）、`website/public/favicon.svg`（品牌蓝鲸身）与 `apps/web/public/favicon.svg`（深色模式转白，pwa-manifest.e2e.ts 锁定该行为）。不要再用 `tauri icon` 单母版再生成——它会把小尺寸帧覆盖回细节版。眼睛射线必须用 `<polygon>` 而非 `<path>`，否则 apps/web 深色模式的 `path { fill: #fff }` 会把它漂白。`src-tauri/icons` 只提交被 `tauri.conf.json` 引用的文件（icon.icns/icon.ico/32x32/128x128/128x128@2x）。改图标后重启应用，Dock 图标缓存才会刷新。
- Windows 兼容：`.cmd` 脚本不能直接 spawn，须经 `%ComSpec% /C`（见 `run_pnpm`）；新增子进程调用保持同样分支。
- 进程回收：内核子进程在 Unix 上 `setsid` 独立进程组，停止时杀整组；应用退出时兜底回收（`lib.rs` 的 `RunEvent::Exit`）。新增后台进程沿用该模式。
- 版本发布由 `.github/workflows/desktop-release.yml` 负责（tag `desktop-v<version>` 触发）；发版前同步 `package.json` 与 `tauri.conf.json` 的 `version`。

## 已知坑

| 症状 | 处理 |
| --- | --- |
| `pnpm install` 在 desktop/ 内装出整个 monorepo | 忘了 `--ignore-workspace` |
| Tauri 同步命令里创建 webview 卡死 | 用新线程创建（`open_harness` 模式） |
| macOS 访问 `127.0.0.1:3080` 失败 | WKWebView 默认允许环回，勿加 `NSAppTransportSecurity` 例外 |
| 编辑器报 `capabilities/default.json` 缺 `$schema` | schema 由首次 `tauri build` 生成，属正常 |
