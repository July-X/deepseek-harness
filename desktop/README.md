# DeepSeek Harness 桌面端（dsh-desktop）

一个基于 [Tauri v2](https://tauri.app/zh-cn/) 的桌面外壳：在桌面上自由打开 DeepSeek Harness 的 Web UI，并提供**内核更新菜单**——跟随官方
`deepseek-ai/deepseek-harness` 的 GitHub Release tag（`dsh-v*`）一键安装、切换、删除、更新内核版本。

> 本目录是独立交付物，**不加入**仓库的 pnpm workspace，不会参与上游的 lint / hygiene / release 门禁。

## 它如何工作

```
┌────────────────────────── dsh-desktop (Tauri v2) ──────────────────────────┐
│                                                                             │
│  主窗口（管理面板）       ┌──────────────────────────────────────────────┐  │
│  ui/ 静态页面             │  Harness 窗口 (WebviewWindow "harness")      │  │
│  · 内核状态/启动/停止      │  加载 http://127.0.0.1:<port>               │  │
│  · 更新菜单（版本管理）    │  = dsh web 的 Web UI                        │  │
│  · 设置 / 日志             │                                              │  │
│            │ invoke               ▲                                      │  │
│            ▼                     │                                       │  │
│  ┌────────────────────────────────────────────────────────────┐          │
│  │ Rust shell：内核生命周期 + 版本管理                           │          │
│  │ · 安装：pnpm add @deepseek-ai/dsh@<版本>                     │          │
│  │   （node-linker=hoisted，实时日志流式回传 UI）                 │          │
│  │   → ~/.dsh/desktop/kernels/<version>/                       │          │
│  │ · active 指针：~/.dsh/desktop/active.txt                    │          │
│  │ · 启动：node …/lib/bin.js web --no-open --port 3080        │          │
│  └────────────────────────────────────────────────────────────┘          │
│                       ▲                                                   │
│   内核数据（会话/设置）  │ 独立于外壳，在 ~/.dsh                          │
└───────────────────────┴───────────────────────────────────────────────────┘
```

- **内置访问方式**：外壳在本地启动 `dsh web` 服务并用专用窗口加载其 Web UI，无需手动打开浏览器。
- **内核更新**：官方发布到 npm registry 的 `@deepseek-ai/dsh`（以及同名 `dsh-*` 依赖包）页面 [`https://www.npmjs.com/package/@deepseek-ai/dsh`](https://www.npmjs.com/package/@deepseek-ai/dsh) 与 GitHub `dsh-v<semver>` tag 一一对应；更新菜单直接读 npm registry（`https://registry.npmjs.org/@deepseek-ai/dsh`）拿到全量版本与 `dist-tags`，可安装、切换、删除任意已发布版本；只有 npm registry 不可达时才回退 GitHub Releases API 与其 Atom feed。

## 功能

- 一键启动 / 停止 / 打开 Harness 工作台
- 更新菜单：列出 npm registry [`@deepseek-ai/dsh`](https://www.npmjs.com/package/@deepseek-ai/dsh) 的所有发布版本（含预发布标记），安装、切换活动版本、删除本地版本
- 内核安装通过 pnpm 执行（`node-linker=hoisted` 保持扁平 `node_modules`，内容寻址存储让重复安装更快），安装过程逐行流式显示在进度面板中，完整日志落盘 `~/.dsh/desktop/logs/install-<版本>.log`
- Node.js 自动检测与手动指定（要求 `^22.19 || >=24`，与 dsh 的 engines 一致）
- pnpm 路径可配置（默认取 node 同目录或 PATH）
- 端口可配置（默认 3080）
- 内核运行日志查看；应用退出时自动回收内核子进程
- **插件管理**：社区插件（npm 包或 GitHub 仓库）统一存入 `~/.dsh/plugins/`，以**链接**（默认，Windows 自动降级**复制**）的方式进入每个已安装内核（`~/.dsh/desktop/kernels/<版本>/plugins/`），并自动接线进 profile——切换内核无需重装；「插件中心」对接 [dsh-plugin-hub](https://dsh-plugin.org) 目录（分类/搜索/排序/已安装过滤，6 小时本地缓存，官方 market 兜底），安装前校验 dsh 规范；管理面板提供安装/卸载/更新/切换模式/同步，检测到新版本时在卡片与启动时提醒
- **技能管理**：社区技能（npm 包 / GitHub 仓库 / 本地文件夹）统一存入 `~/.dsh/skills-store/`，按「包安装、单技能启停」的粒度以链接（失败降级复制）物化进内核自带扫描的 `~/.dsh/skills/`——不改 cordis 配置、不装依赖、切换内核零操作；内核对技能根做文件监视，**启用/停用/卸载/更新对运行中的工作台即时生效，无需重启**；安装前逐个校验 SKILL.md frontmatter（kebab-case `name` + `description` 必填），避免"装了却不出现"；本地文件夹来源支持改完点「重新同步」；启动时自动对账（补链、清扫孤儿链接、恢复中断的更新）

## 目录结构

```text
desktop/
├── package.json              # 仅 @tauri-apps/cli 脚本；不进 pnpm workspace（pnpm-lock.yaml 独立管理）
├── ui/                       # 管理面板（零构建静态前端）
│   ├── index.html
│   ├── styles.css
│   ├── plugins.js            # 插件管理卡片（安装/更新/目录搜索/更新提醒）
│   ├── skills.js             # 技能管理卡片（包安装/单技能启停/目录搜索/即时生效提示）
│   ├── whale-icon.png        # 顶栏 logo（60 CSS px 显示，故由 assets/whale-icon-small.svg 渲染 128px）
│   └── app.js
├── docs/
│   ├── plugin-management.md  # 插件管理设计（目录布局、链接/复制双模式、接线、更新机制）
│   └── skill-management.md   # 技能管理设计（中央库 + 物化到 ~/.dsh/skills，热生效，无需接线）
├── assets/                   # 全仓库图标母版（scripts/build-icons.sh 由此再生成一切）
│   ├── whale-icon.svg        # 完整细节母版（黑鲸 + 红眼，用于 ≥128px）
│   ├── whale-icon-small.svg  # 小尺寸母版（红眼夸大版，用于 ≤64px 与 favicon 投影）
│   └── whale-icon-512.png    # 512px 位图（脚本从 whale-icon.svg 渲染）
├── scripts/
│   └── build-icons.sh        # 从双 SVG 母版再生成全部图标：src-tauri/icons、ui/whale-icon.png、website 与 apps/web 的 favicon
├── src-tauri/
│   ├── tauri.conf.json       # Tauri v2 配置（frontendDist → ../ui）
│   ├── Cargo.toml
│   ├── capabilities/default.json
│   ├── icons/                # 应用图标集（黑鲸+红眼，由 assets 管线生成并提交）
│   └── src/
│       ├── main.rs / lib.rs  # 入口与装配（含退出时回收内核）
│       ├── commands.rs       # Tauri 命令（含插件/技能命令与切换/启动接线钩子）
│       ├── kernel.rs         # 安装 / active / 启动 / 停止 / 端口探测
│       ├── plugins.rs        # 插件中央库、内核物化（link/copy）、profile 接线、更新检查、社区目录
│       ├── skills.rs         # 技能中央库、单技能物化到 ~/.dsh/skills（link/copy）、启停、更新检查、目录、启动对账
│       ├── releases.rs       # 官方发布列表（API + Atom 回退 + semver 排序）
│       ├── node.rs           # Node/pnpm 检测与版本校验
│       └── settings.rs       # settings.json 读写（含 profile 名）
```

## 本地构建

前提：Rust 工具链（含 `cargo`）、Node.js 22+；`scripts/install.mjs` 会自动检测 pnpm，缺失时回退到 npm。

```sh
cd desktop

# 安装 Tauri CLI（自动检测 pnpm，缺失时回退到 npm；带 --ignore-workspace 防向上匹配根 pnpm-workspace.yaml）
npm run deps

# 开发运行（需先安装内核，见「使用」）
npm run dev

# 本机当前架构构建
npm run build

# 指定目标平台
npm run build:mac-intel   # x86_64-apple-darwin（Intel Mac）
npm run build:win         # x86_64-pc-windows-msvc
```

> 想直接走 pnpm / npm 也行：`pnpm install --ignore-workspace` 或 `npm install --ignore-workspace` 同样有效；wrapper 只是省略记忆参数。

产物位于 `src-tauri/target/release/bundle/`（macOS 为 `.dmg`，Windows 为 NSIS 安装包 `.exe`）。

## 使用

1. 启动桌面应用，打开管理面板。
2. **设置**：确认已检测到满足要求的 Node.js（不满足时安装 Node 22.19+ 或手动指定路径）。
3. **内核更新** → 点击「检查更新」→ 在官方发布列表中选择版本点「安装」。
   - 安装通过 pnpm 执行，进度面板会实时滚动 pnpm 日志；pnpm 未安装时按提示 `npm install -g pnpm` 或在设置中指定 pnpm 路径。
   - 首次安装会自动成为活动版本并自动启动内核。
   - 之后安装的版本不会覆盖当前活动版本，可随时「切换」或「删除」。
4. （可选）**插件** → 在「插件中心」按分类浏览、搜索（即时过滤）、按 Star/更新时间排序后一键安装，或手动填写 npm 包名（如 `@ace-zone/dsh-market`）/ GitHub 仓库 URL 安装；安装前自动校验插件是否符合 dsh 规范（package.json / `dsh.bundle.patch` / 入口文件），安装完成后重启工作台（关闭后重新启动）生效。
5. 在「概览」页点击「启动工作台」：自动拉起内核、等待就绪后打开工作台窗口进入 Harness 界面；启动失败会自动弹出内核日志。「关闭工作台」会同时关闭工作台窗口并停止内核；内核运行中窗口被关掉时，可用「打开工作台窗口」重新打开。
6. 首次使用时在 Harness 的设置页配置 DeepSeek（`DEEPSEEK_API_KEY` 等）即可开始对话。


数据目录（统一在 dsh home 下的 `desktop/` 二级目录）：
- 外壳元数据（已装版本、活动指针、设置、日志）：`~/.dsh/desktop/`（`kernels/`、`logs/`、`settings.json`、`active.txt`；可用 `DSH_HOME` 环境变量重定向整个 dsh home）
- 内核数据（会话、配置、profile）：`~/.dsh`
> 从旧版本升级：旧版外壳把元数据存在系统应用数据目录（macOS `~/Library/Application Support/com.zhongxingxing.dsh-desktop/`）。新版启动后该处数据不再读取，请将旧目录下的 `kernels/`、`logs/`、`settings.json`、`active.txt` 手动移到 `~/.dsh/desktop/`。

## 发布（GitHub Actions）

工作流：[`.github/workflows/desktop-release.yml`](../.github/workflows/desktop-release.yml)

- 支持平台：**Intel macOS**（`macos-13`，`.dmg`）+ **Windows x86_64**（`windows-latest`，NSIS `.exe`）
- 触发方式：
  - 推送 tag：先同步 `desktop/package.json` 与 `src-tauri/tauri.conf.json` 的 `version`，再 `git tag desktop-v<version>` 并推送；或
  - 手动在 Actions 页触发 `workflow_dispatch`（使用当前 `package.json` 版本）。
- 产物发布为 **draft release**，经人工确认后正式发布。
- 多平台矩阵并行构建，同一 tag 重复运行会向同一个 draft release 追加产物。

> 签名说明：当前产物未做代码签名，Windows SmartScreen 与 macOS Gatekeeper 可能给出警告。加入签名（Apple Developer ID / Windows 代码签名证书 + 对应 secrets）后再去掉相关提示。

## 常见启动失败与处理

| 症状 | 排查 |
| --- | --- |
| `WebviewWindowBuilder` 创建工作台窗口卡死 | Tauri 2.x 在同步命令里创建 webview 窗口**会死锁**（Windows 100%；macOS/Linux 部分情况下也慢）。本项目 `open_harness` 已经把创建放在新线程（`commands.rs::open_harness`）。新增类似命令请保持同样模式。 |
| macOS 启动后访问 `http://127.0.0.1:3080` 失败 | Tauri 2.x 默认 WKWebView 已允许本地环回访问，不需要 `NSAppTransportSecurity` 例外；本项目移除了该字段，依赖平台默认值。 |
| 编辑器/IDE 报 `capabilities/default.json` 找不到 `$schema` | schema 文件在首次 `tauri build` 后由 `tauri-build` 生成；本项目移除了硬编码 `$schema` 引用，避免初次克隆时编辑器红字。 |
| 升级后「已安装」列表为空 | 外壳元数据已迁到 `~/.dsh/desktop/`；按上文“数据目录”提示迁移旧目录内容，或重新安装内核。 |

## 已知限制与后续

- **Node 运行时**：目前检测系统 Node 或手动指定；后续可捆绑 Node sidecar 实现开箱即用（分发体积 +40MB/平台）。
- **pnpm 依赖**：内核安装依赖用户环境中的 pnpm（未捆绑）；后续可评估 `corepack` 或 sidecar 方式随应用分发。
- **端口冲突**：若 3080 已被其他进程占用，先停止外部服务或改端口。
- **安全**：应用通过 Webview 加载本地 `http://127.0.0.1` 的 Harness 页面并暴露版本管理命令；仅信任官方 `deepseek-ai` 仓库与 npm 的 `@deepseek-ai` 命名空间。插件是第三方任意代码，安装前请自行确认来源；社区目录条目保留「未验证」标记。
- **插件链接模式**：依赖文件系统符号链接支持（Windows 需要开发者模式，失败会自动降级为复制模式并在行内显示「复制」徽标）。
- **自动更新**：桌面应用自身的升级可后续接入 tauri-plugin-updater；当前发布流程聚焦内核更新。
