# Agent Note：用于内核管理的 Tauri 桌面外壳

Status: implemented

[English](2026-08-21-tauri-desktop-shell.md) | 中文

## 问题

DeepSeek Harness 目前的消费方式是通过浏览器 UI 使用：`npx @deepseek-ai/dsh web`（或 `pnpm dsh web`）在 `http://127.0.0.1:3080` 启动。用户需要一个桌面外壳：(1) 在桌面上自由打开 harness；(2) 提供一个内核更新菜单，跟随官方 `deepseek-ai/deepseek-harness` 的 Release tag 原位安装与切换内核版本；(3) 通过 GitHub Actions 面向 Intel macOS 与 Windows 发布。官方 GitHub Release 只附带源码包；真正的内核以 `@deepseek-ai/dsh` npm 包（连同数十个 `@deepseek-ai/dsh-*` 依赖包）分发，版本与 `dsh-v<semver>` tag 一一对应。因此桌面端无法从 release asset 直接拿到"内核"——必须安装 pinned 到该 tag 版本的 npm 包。

## 决定

**外壳是独立于 pnpm workspace 的 `desktop/` 目录树。**它从不进入 `pnpm-workspace.yaml`，于是上游的锁文件、lint/hygiene/release 序列和打包门禁都不会触及它，也不会被它破坏不变量。它是独立的交付物，带自己的 `package.json`（仅 `@tauri-apps/cli`）和自己的 Rust crate。

**Tauri v2 承载两个窗口。**主窗口加载零构建的静态 `ui/` 资源（本地管理面板：状态、更新菜单、设置、日志）。第二个 `WebviewWindow`（label 为 `harness`）加载 `http://127.0.0.1:<port>`——即 `dsh web` 服务——让 harness UI 在独立窗口运行。没有前端构建步骤；`frontendDist` 指向 `ui/`，`withGlobalTauri` 为面板的纯 JS 暴露 `window.__TAURI__.core` 桥。

**内核版本是钉在应用数据目录下的 npm 安装。**`fetch_releases` 通过 GitHub REST API 读取官方发布列表，API 被限流时回退到 releases Atom feed（tag 相同；回退仅丢失 prerelease 标记）。安装某版本执行 `npm install --prefix <app_data>/kernels/<version> @deepseek-ai/dsh@<version>`；活动版本是一个纯文本文件 `<app_data>/active.txt`。首次安装自动激活并自动启动；后续安装不动当前活动版本。因此"更新菜单替换内核"的方式是钉住另一版本并切换活动指针，而不是修补既有安装。

**Node 来自环境、可配置。**外壳从设置、再到 PATH、再到常见安装位置解析 `node`，并按 dsh 的 engine 范围（`^22.19 || >=24`）校验；npm 依次从设置、`node` 同目录、PATH 解析。这让 POC 免于捆绑 Node sidecar。

**发布是专用的 workflow。**`.github/workflows/desktop-release.yml` 用 `tauri-apps/tauri-action` 在 `macos-13`（Intel macOS，`.dmg`）与 `windows-latest`（`.exe`/NSIS）构建，由 `desktop-v*` tag 或手动触发，产出草稿 release 交给人工发布。

**内核子进程在退出时被回收。**Unix 上它以独立会话启动（`setsid`），停止时信号发给整个进程组；Windows 上用 `taskkill /T /F` 拆除整棵进程树。

## 曾考虑的替代方案

**把 `apps/desktop` 放进 pnpm workspace。**否决：仓库的包约束、lint 面和发布/打包序列都会扫描 workspace 成员；把 Rust/Tauri 交付物放进去会把外来工具链拖进上游门禁，其 `package.json` 还得满足 workspace 不变量。独立 `desktop/` 目录树是 fork 自有产品爆炸半径最小的归宿。

**捆绑 Node sidecar 做到开箱即用。**POC 否决：每个平台需要各自的 Node 二进制（每目标 +40 MB），外壳还得自举它，而且本地环境完全无法验证 Rust 构建。环境检测 node + 可配置路径是可逆的默认；sidecar 槽位留作后续项记录在文档。

**从 GitHub release 拉源码 tarball 当内核。**否决：桌面应用不可能在用户机器上跑 `pnpm install` + 整套 harness 构建；发布契约本来就在 npm，一个 `dsh-v*` tag 对应一个版本。

## 后果

内核数据（会话、设置、profile）留在 dsh 自己的 `~/.dsh` home；只有外壳元数据（已装版本、活动指针、设置、日志）位于应用数据目录。GitHub REST 回退到 Atom 只在更新菜单里以警告形式可见。安装包未签名，因此 Windows SmartScreen 与 macOS Gatekeeper 会告警，直到加入代码签名。工作台窗口与管理面板是两个窗口；面板是管理内核的唯一入口，每次刷新状态都通过探测端口自动恢复陈旧状态。

## 测试

配置文件 JSON 已用 `python3 -m json.tool` 校验。应用图标基于官方 DeepSeek favicon（`desktop/assets/whale-favicon.svg`，黑色身体，取自 `https://fe-static.deepseek.com/platform/favicon.svg`），以矢量圆方式加了一只红色眼睛（成品 `desktop/assets/whale-icon.svg`）；`src-tauri/icons/` 位图集由该 SVG 的 512px 浏览器栅格化结果（`desktop/assets/whale-icon-512.png`）降采样生成，并用 `file` 校验。更新菜单的版本列表现在来自 npm registry（`https://registry.npmjs.org/@deepseek-ai/dsh`，对应用户访问的 `https://www.npmjs.com/package/@deepseek-ai/dsh` 包页面）；`fetch_npm` 解析 `versions` 与 `dist-tags`，prerelease 标记与发布时间都是权威数据。GitHub Releases (`fetch_api`) 与其公共 Atom feed (`fetch_atom`) 仅在 registry 不可达时作为 fallback；UI 通过 `warning` 字段提示当前正在用哪条数据源。当前代码显式避开了四个 Tauri 2.x 启动陷阱：`WebviewWindowBuilder` 由 `open_harness` 在新 OS 线程中创建，避免在同步命令里卡死 Windows 主线程；`macOS.infoPlist.NSAppTransportSecurity` 被省略，使用平台默认本地网络策略；`capabilities/default.json` 不硬编码尚未生成的 `$schema` 路径；`ureq` 启用 `rustls` feature（3.x feature 名是 `rustls` 而非 `tls`），避免引入系统 TLS 依赖。同时修复了若干编译/运行错误：`commands.rs::install_kernel` 把 `data_dir` / `version` 在送入 `spawn_blocking` 闭包前先 `clone`，这样 join 后仍能用它们做后续 `set_active`/自动启动；`commands.rs` 给 `Channel<String>::send` 传 `String` 而非 `&str`；`kernel.rs::port_open` 直接构造 `SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127,0,0,1), port))`，因为 `(&str, u16)` 不实现 `Into<SocketAddr>`；`releases.rs` 用 ureq 3.x 的 `.header(...)`（不是 `.set`）并给每个 `map_err` 闭包显式标注 ureq 错误类型；`node.rs::parse_version` 三个数字段都用 `parse::<u32>()`；`kernel.rs::start` 把 `port.to_string()` 提前为 `String` 再传入 `cmd.arg(...)`；`releases.rs::parse_atom` 把切片偏移始终锚定在原 buffer 上，避免多字节 UTF-8 标题把 `&str` 切片切到字符边界之间导致 panic。代码 review 轮的四项健壮性修复：`run_pnpm` 改用 `recv_timeout(10s)` 接收，依赖解析静默期发「已进行 N 秒」心跳而不是看起来卡死；内核 pid 在 spawn 时写入 `<data_dir>/kernel.pid`，`stop_kernel` 与 `RunEvent::Exit` 钩子都会在端口仍被占用时回退到 `kill_pid`（先用 ps 校验命令行身份防 pid 重用误杀），崩溃后遗留的孤儿内核也能回收；`write_active` 先写 `active.txt.tmp` 再 rename、删除时容忍文件不存在，崩溃不会截断活动指针；`rotate_install_logs` 在每次安装前只保留最近 9 份安装日志。本地环境没有 Rust 与 Node 工具链，因此 `cargo build` 与 Tauri 装配由 CI 中的 `desktop-release` workflow 验证；更新菜单的发布列表在装有 Node 并构建出外壳后，于应用内直接对真实 npm registry（以及 GitHub fallback）完整跑一遍。
