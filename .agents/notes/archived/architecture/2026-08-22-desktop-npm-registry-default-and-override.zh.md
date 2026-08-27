# Agent Note: dsh-desktop 把全部 npm 流量统一到一个 registry，默认走 npmmirror 镜像

Status: implemented
Archived: 2026-08-28

[English](2026-08-22-desktop-npm-registry-default-and-override.md) | 中文

## 问题

桌面外壳从 npm 生态拉取依赖的入口分散在五处——内核安装、插件库依赖、profile 接线、git 来源插件的 `prepare` 构建、tarball 下载——另外还有两处直接发 HTTP 的 ureq 请求（版本列表、插件元数据）。改之前每个入口都把 `https://registry.npmjs.org/` 直接写成字符串字面量，少数情况还会落到写盘的 `*.npmrc` 里。结果就是同一个上游在中国网络下反复卡人：内核刚装上，到了插件环节又卡在同一个源上。更棘手的是没有覆盖点：要换 registry 必须改源码重新构建，CI 或海外部署根本动不了。

## 决策

**单点真相落在 `desktop/src-tauri/src/registry.rs`。** 一个 `DEFAULT_NPM_REGISTRY = "https://registry.npmmirror.com/"` 常量加上 `npm_registry_base()` 解析器，统共一个模块。其它所有入口都从这里读，仓库里不再保留任何 registry URL 字面量。

**三处注入点分层叠加，互为兜底。**

1. **每个子进程都带环境变量**——`process::spawn` 是 `pnpm` / `npm` / `pnpm.cmd` 这类命令的唯一漏斗，在 `Command` 上注入 `npm_config_registry`。它是 pnpm 和 npm 都会读取的最高优先级来源，盖得过项目级和用户级 `.npmrc`。外壳不需要动任何 `.npmrc` 就能强制走镜像。
2. **插件库里也写一份 `.npmrc`**——`plugins::ensure_store_npmrc` 现在把同样的 registry 值写到 `~/.dsh/plugins/.npmrc` 里，替换原来的硬编码 npmjs.org URL。这个文件本来就要写（用来关掉 pnpm 的 `minimumReleaseAge`），顺便把 registry 也带进去。如果某个 pnpm 子进程没继承到环境变量，至少还能从这份 `.npmrc` 解析到镜像，scope 包（`@deepseek-ai/*`）走的是对应的 scope 行，顺带解决。
3. **HTTP 抓取用同一个 base**——`releases::fetch_npm`（内核版本列表）和 `plugins::fetch_npm_doc`（插件元数据）都用 `registry::npm_registry_base()` 拼 URL。tarball URL 是从元数据文档里读出来的，自动跟着选中的 registry 走，不用再单独处理。

**覆盖机制：`DSH_NPM_REGISTRY` 环境变量。** 设置成非空非空白字符串时替换默认值。解析器每次调用都规范化（trim、强制末尾斜杠），调用方可以统一 `format!("{base}{pkg}")` 拼 URL。不设就走默认，对绝大多数用户完全无感。

## 后果

五个 pnpm 调用点、两处 ureq HTTP 路径、写盘的插件库 `.npmrc`，现在都从同一个 base URL 出发。中国网络下用户不用配置全局 `~/.npmrc` 就能装，需要上游 registry 的运维方设一次 `DSH_NPM_REGISTRY=https://registry.npmjs.org/` 就行。版本列表的 GitHub 回退链路（`fetch_api` → `fetch_atom`）原样保留，所以镜像挂了还能优雅地降级到 GitHub，沿用原有的 warning 提示——只是主源换了。

五个单元测试覆盖了解析器（`registry::tests::*`）：默认值、覆盖、trim、末尾斜杠强制、空白回退。`resolve` 是从 `npm_registry_base` 里拆出来的纯函数，测试不用动进程全局环境。

明确放弃的能力：没有逐调用点的覆盖。也没有 UI 开关去运行时切换 registry，覆盖方式是进程启动时的环境变量。这是故意的——这是个 GUI 应用不是 CLI，部署相关的选择放在启动环境里比放在设置面板里更自然。这个接缝（`registry::npm_registry_base`）小且稳定，将来要做 UI 控制也只需要改一个函数。

没动的部分：GitHub Releases 回退 URL（`releases.rs`）、dsh-plugin.org 目录端点、平台发布产物管线、内核子进程 `dsh web`（它跑的是已经装好的 `node_modules`，不会再去解析 npm）。这些都不碰 npm。

## 备选方案

- **每个 pnpm 调用都加 `--registry` CLI flag**——同一个字符串在五个地方重复写，ureq 那两条路径还覆盖不到。达不到"单点真相"的目标。
- **只依赖用户全局 `~/.npmrc`**——对已经配好镜像的用户有效，但没配的用户就完全帮不上，而且没法强制执行。这次改动的初衷就是不依赖用户配置。
- **硬编码镜像不做任何覆盖**——代码更简单，但 CI / 海外部署得改源码重新构建。一个环境变量不增加任何成本，解析器的规范化（trim、末尾斜杠、空白回退）也就多两行 `unwrap_or_else`。
- **设置面板里加一个开关取代或并用环境变量**——对一个部署相关的选择来说表面积太大。shell 的 `Settings` 结构是为用户稳定偏好（node 路径、端口、profile）准备的；registry 是运维策略，跟随启动环境。等真有人提这个需求再设计 schema 不迟。
- **干脆砍掉 npm HTTP，只走 GitHub Releases 回退**——能回避覆盖点的设计问题，但版本列表会丢掉 `dist-tags`、`time`、prerelease 标记这些更新菜单需要的数据。属于范围外的事，不在这次镜像切换里做。
