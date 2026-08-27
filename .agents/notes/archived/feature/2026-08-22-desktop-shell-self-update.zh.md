# Agent Note: 桌面端自更新（tauri-plugin-updater）

Status: implemented
Archived: 2026-08-28

[English](2026-08-22-desktop-shell-self-update.md) | 中文

## 问题

桌面端能更新*内核*（npm 版本），但对*自身*更新没有答案：概览页连桌面端自己的版本号都不显示，用户只能自己盯 GitHub 发布页。更新流程需要覆盖自动发现、手动检查、以及应用内安装并替换当前应用。

## 决策

**自更新走 `tauri-plugin-updater`，数据源是 GitHub releases。** 发布 workflow 用仓库 secret `TAURI_SIGNING_PRIVATE_KEY` 给更新制品（`latest.json` + `.sig` 文件）签名；`desktop/src-tauri/tauri.conf.json` 里钉死的 `pubkey` 让客户端拒绝任何未经该密钥签名的载荷。`bundle.createUpdaterArtifacts: true` 让两个平台都产出签名制品。

**endpoint 指向 latest 已发布 release。** `https://github.com/July-X/deepseek-harness/releases/latest/download/latest.json` 只服务已发布的 release——draft 不可见，正好契合人工发布环节。rc 预发布版也可以在 GitHub 界面上被标记为 latest，需要让 rc 用户收到下一次更新时这么操作即可。

**发现 = 推送 + 拉取。** `updater::spawn_background_check` 在启动约 3 秒后检查一次并发出 `shell-update-available` 事件；概览页同时显示当前运行版本（`StatusView.shell_version`，来自 `app.package_info()`）并提供「检查桌面端更新」按钮做手动检查。`install_shell_update` 通过 Channel 流式回报进度，完成后 `app.restart()`。

**NSIS 保持 `currentUser`。** 只有安装器从未要求过提权，更新器才能原地替换应用而不弹 UAC。

## 备选方案

**查 releases API 再手动拉起安装器。** 否决：没有签名校验（中间人或仓库被攻破就能给所有桌面端投任意代码），还要自维护两条平台相关的替换路径，且没有原子替换。updater 插件是被维护的标准路径，签名校验是安全底线。

**在 GitHub releases 之外自架 JSON endpoint。** 否决：多一份要托管和保持同步的制品；release 自带的 `latest.json` 不可能和它描述的产物漂移。

## 后果

- 丢失 `TAURI_SIGNING_PRIVATE_KEY`（本地保存在 `~/.tauri/dsh-desktop.key`，另存于 fork 的 secrets）意味着更新无法通过校验；轮换方式是生成新密钥对，同时更新钉死的 pubkey 和 secret。
- 版本与 tag 的一致性（`desktop-v<version>`）是更新版本可比较的前提；workflow 的 verify 步就是这道护栏。
