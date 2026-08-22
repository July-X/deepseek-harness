# Agent Note：模型思考与回复语言

状态：已实现

[English](2026-08-22-response-language-for-model-thinking.md) | 中文

## 问题

模型的思考与回复跟随提示词本身的语言。系统提示完全由英文构成——harness 身份开场白、harness 源码定位、工具 schema 与引导、plan-mode 与 persona 文本——没有任何指令钉住用户语言。因此推理模型对中文用户产出英文思考步骤，而界面语言偏好（`dsh-client-locale` 写入 Host 用户设置文档的 `locale.preference`）从未进入模型输入。

## 决策

`dsh-system-prompt` 配置新增 `responseLanguage?: string`（schema 默认 `''`）。非空值会在 −100 身份开场白与 0 persona 之间注册一个固定顺序 −98 的 `harness:language` 段：

```

Think and reply in ${language}. Write every reasoning and thinking step in ${language} as well.

```

空值不渲染，未配置的部署提示词逐字节不变。

base bundle（`packages/bundle/base/cordis.patch.yml`）在 `persona` 旁设置了产品默认 `responseLanguage: 简体中文`，与已交付的中文界面一致；界面语言不同的 profile 通过整行覆盖（patch 替换整行）。`dsh-agent-spine-demo` 通过其 `Config` 接口、`pickSpineConfig` 与 `SystemPrompt` 子插件转发该键；其 schema 与各 owner schema 求交，因此该键无需单独的 schema 条目。

段文本按配置固定，提示词前缀保持稳定，利于 KV-cache 复用；每次请求只增加一个句子。

## 备选方案

- **只提供配置能力、不给默认值** —— 所有默认部署仍不解决用户可见的问题。
- **把中文指令硬编码进 harness 身份** —— 对英文界面用户是错误的；产品同时交付 zh 与 en。
- **从 Host 设置读取 `locale.preference` 并动态注册该段** —— 持久化偏好已存在，但 host 端读取链路与变更传播超出本次范围；未来该段改用 `text` 提供方即可采纳，无需契约变更。

## 影响

- 默认 web/CLI 会话（base bundle）要求以简体中文思考和回复（含推理步骤）；这是模型可见输入，已记录快照会变更一次。
- 部署与 profile 通过同一 system-prompt 行覆盖；空值恢复原行为。
- 服务与 bundle README、生成的配置目录及其中文对侧均记录该键。

## 测试

system-prompt 套件钉住段顺序（身份 → 语言 → persona）、精确渲染文本与默认缺席。spine-demo 与 base-bundle 套件覆盖转发与 patch 可解析性；配置目录在本次变更中重新生成（`pnpm run gen-config-catalog`）并重录中文对侧配对（`pnpm run verify-translation-pairing --write docs/config-catalog.md`）。
