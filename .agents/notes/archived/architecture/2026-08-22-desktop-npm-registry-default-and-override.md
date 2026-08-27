# Agent Note: dsh-desktop pins every npm flow to one registry, defaulted to the npmmirror mirror

Status: implemented
Archived: 2026-08-28

English | [中文](2026-08-22-desktop-npm-registry-default-and-override.zh.md)

## Problem

The desktop shell pulls from the npm ecosystem through five independent call sites (kernel install, plugin store deps, profile wiring, git-origin plugin `prepare` builds, tarball downloads) plus two ureq HTTP fetches (release listing, plugin metadata). Before this change each site hard-coded `https://registry.npmjs.org/` either directly or as a string literal in a `*.npmrc` it wrote to disk, so a user on a constrained network could install a kernel only to have the plugin step hang on the same upstream. There was no override seam: changing the registry meant editing source, and CI / overseas deployments had no way to flip it without rebuilding.

## Decision

**One source of truth: `desktop/src-tauri/src/registry.rs`.** A single `DEFAULT_NPM_REGISTRY = "https://registry.npmmirror.com/"` constant and a `npm_registry_base()` resolver live in one module. Every other call site reads through it; nothing else holds a registry URL literal.

**Three coordinated injection points, layered for defense in depth.**

1. **Environment variable on every child process** — `process::spawn` (the single funnel for `pnpm` / `npm` / `pnpm.cmd` and friends) injects `npm_config_registry` on the `Command`. This is the highest-priority source both pnpm and npm consult, and it survives every existing project- or user-level `.npmrc` that might pin something else. The shell does not need to mutate those files to enforce the mirror.
2. **`.npmrc` written into the plugin store** — `plugins::ensure_store_npmrc` now writes the same registry value into `~/.dsh/plugins/.npmrc` instead of a hard-coded npmjs.org URL. The store-level file already had to exist to disable pnpm's `minimumReleaseAge`; piggy-backing the registry there means a pnpm subprocess that *doesn't* inherit the env var still resolves through the mirror, and scoped packages (`@deepseek-ai/*`) get the matching scope line for free.
3. **HTTP fetches use the same base** — `releases::fetch_npm` (kernel version listing) and `plugins::fetch_npm_doc` (plugin metadata) compose their URLs from `registry::npm_registry_base()`. Tarball URLs come back from the metadata document, so they follow the chosen registry automatically; no second-layer rewrite needed.

**Override mechanism: `DSH_NPM_REGISTRY` environment variable.** When set (non-empty, non-whitespace), it replaces the default. The resolver normalizes the value on every read (trim, force trailing slash) so caller sites can `format!("{base}{pkg}")` uniformly. Default behavior is unchanged for everyone who does not opt in.

## Consequences

All five pnpm call sites, both ureq HTTP paths, and the on-disk store `.npmrc` now route through one base URL. A Chinese-network install works without touching the user's global `~/.npmrc`, and an operator who needs the upstream registry sets `DSH_NPM_REGISTRY=https://registry.npmjs.org/` once. The release list's GitHub fallback chain (`fetch_api` → `fetch_atom`) stays intact, so a mirror outage still degrades gracefully to GitHub with the existing warning message — only the primary source moved. Five unit tests cover the resolver (`registry::tests::*`): default, override, trim, trailing slash enforcement, and whitespace-falls-back. The resolver is a pure function split from `npm_registry_base` specifically so tests do not need to mutate process-global env.

What the change knowingly gives up: a per-call-site override. There is no UI knob to switch registries on the fly; the override is a process-start environment. That is intentional — the shell is a GUI app, not a CLI, and a deployment-varying choice reads more naturally as a launch-time env var than as a settings-panel toggle. The seam (`registry::npm_registry_base`) is small and stable, so a future UI control would only have to update one function.

What was *not* changed: the GitHub Releases fallback URLs in `releases.rs`, the dsh-plugin.org catalog endpoints, the platform release artifact pipeline, and the `dsh web` kernel subprocess (it already runs against an installed `node_modules` and never re-resolves npm). None of those touch npm.

## Alternatives considered

- **Per-call-site `--registry` CLI flag on every pnpm invocation** — duplicates the same string across five sites and offers no override hook for the ureq paths. Fails the "one source of truth" goal.
- **Rely on the user's global `~/.npmrc` only** — works for users who already configured the mirror, but ships nothing for users who haven't and offers no enforcement. The whole point of the change is to not depend on user config.
- **Hard-code the mirror with no override** — simpler code, but CI / overseas deployments would need a source patch and rebuild. One-line env var costs nothing and the resolver's normalization (trim, trailing slash, whitespace-falls-back) is two extra `unwrap_or_else` lines.
- **Surface a settings-panel toggle instead of (or in addition to) the env var** — bigger surface area for a deployment-varying choice. The shell's `Settings` struct is for user-stable preferences (node path, port, profile); the registry is operator policy that follows the launch environment. Deferring a UI control until a real ask appears is cheaper than designing the schema today.
- **GitHub-only fallback for everything (drop the npm HTTP entirely)** — would remove the override knob question but loses the release-list richness (`dist-tags`, `time`, prerelease flags) that the update menu consumes. Out of scope for a mirror switch.
