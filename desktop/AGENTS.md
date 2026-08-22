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
- **plugins.rs**：社区插件管理——中央库 `~/.dsh/plugins/`（store.json + 插件源）、按内核物化（link 默认，失败降级 copy；元数据 `.meta/<id>.json`）、profile 接线（deps + `dsh.profile.bundles`，切换/启动时 `ensure_wiring_quiet` 校正）、更新检查（npm dist-tags / git ls-remote）、社区目录（losebird 市场 registry 缓存 6h）。设计见 `docs/plugin-management.md`。
- **releases.rs**：npm registry 全量版本 + dist-tags；registry 不可达时回退 GitHub Releases API 与 Atom feed。
- **node.rs**：Node 检测与 engines 校验（`^22.19 || >=24`）、pnpm 解析（显式配置 → node 同目录 → PATH）。
- **settings.rs**：`settings.json` 平铺结构（`node_path` / `pnpm_path` / `port`），serde default 兼容缺字段。

## 数据目录

外壳全部状态位于 `<dsh_home>/desktop/`（默认 `~/.dsh/desktop/`，`DSH_HOME` 可重定向），由 `kernel::data_dir` 统一解析并在启动时创建；`lib.rs` 的 `setup()` 必须通过它取目录——不要绕回 `app_data_dir()`。子结构：`kernels/<版本>/`、`logs/`、`settings.json`、`active.txt`。

## 约定

- 用户可见文案用简体中文；错误信息必须包含可操作的下一步（如缺失依赖时给出安装指引）与相关日志路径。
- 长任务失败时进度面板保持打开并展示错误与日志区，用户手动点击「关闭」才收起——不要在 catch 后自动隐藏面板。
- 日志双轨：UI 只显示有限行数的实时流（ANSI 转义在前端剥离）；完整原始输出始终落盘 `<data_dir>/logs/install-<version>.log` 与 `kernel.log`。报错信息引用日志路径。
- 应用图标以 `assets/whale-icon-512.png` 为母版，用 `./node_modules/.bin/tauri icon assets/whale-icon-512.png -o src-tauri/icons` 再生成全套；只提交被 `tauri.conf.json` 引用的文件（icon.icns/icon.ico/32x32/128x128/128x128@2x）。改图标后重启应用，Dock 图标缓存才会刷新。
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
