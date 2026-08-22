# Agent Note: Response language for model thinking and replies

Status: implemented

English | [中文](2026-08-22-response-language-for-model-thinking.zh.md)

## Problem

Model thinking and replies followed the prompt's own language. The system prompt is entirely English — the harness identity opener, the harness-source line, tool schemas and guidance, the plan-mode and persona text — and no instruction pinned a user language. Reasoning models therefore produced English thinking steps for Chinese-speaking users, while the UI language preference (`locale.preference` in the Host user-settings document, written by `dsh-client-locale`) never reached model input.

## Decision

`dsh-system-prompt` config gains `responseLanguage?: string` (schema default `''`). A non-empty value registers a fixed order −98 section `harness:language` between the −100 identity opener and the 0 persona:

```

Think and reply in ${language}. Write every reasoning and thinking step in ${language} as well.

```

Empty renders nothing, so deployments that do not configure it keep byte-identical prompts.

The base bundle (`packages/bundle/base/cordis.patch.yml`) sets the product default `responseLanguage: 简体中文` beside `persona`, matching the shipped Chinese UI; a profile with another UI language overrides the whole row (patches replace rows). `dsh-agent-spine-demo` forwards the key through its `Config` interface, `pickSpineConfig`, and the `SystemPrompt` child; its schema intersects the owners' schemas, so the key needed no separate schema entry there.

The section text is fixed per configuration, keeping the prompt prefix stable for KV-cache reuse; it costs one sentence per request.

## Alternatives considered

- **Config-only capability without a default** — leaves the user-facing problem unfixed on every default deployment.
- **Hard-coding a Chinese instruction into the harness identity** — wrong for English UI users; the product ships both zh and en.
- **Reading `locale.preference` from Host settings and registering the section dynamically** — the durable preference exists, but the host-side read chain and change propagation were out of scope; a future `text` provider on this section can adopt it without a contract change.

## Consequences

- Default web/CLI sessions (base bundle) instruct thinking and replies in Simplified Chinese, reasoning steps included; this is model-visible input and changes recorded snapshots once.
- Deployments and profiles override via the same system-prompt row; an empty value restores the previous behavior.
- Service and bundle READMEs, the generated config catalog, and its Chinese counterpart document the key.

## Testing

The system-prompt suite pins section order (identity → language → persona), the exact rendered instruction, and default absence. The spine-demo and base-bundle suites cover forwarding and patch parseability; the config catalog is regenerated (`pnpm run gen-config-catalog`) and its Chinese side re-paired (`pnpm run verify-translation-pairing --write docs/config-catalog.md`) in the same change.
