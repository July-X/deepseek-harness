# Agent Note: Desktop shell manages community skills

Status: implemented
Archived: 2026-08-28

English | [中文](2026-08-24-desktop-shell-skill-management.zh.md)

## Problem

The desktop shell let users manage community **plugins** through a central store, per-kernel materialization, and profile wiring — but community **skills** had no equivalent. Skills are instruction data (`SKILL.md` or flat Markdown with frontmatter) consumed by `dsh-skill-filesystem` from fixed discovery roots at `DSH_HOME` and friends; without a shell-managed path the user had to clone repos by hand into the right directory and restart the workbench to pick up the watcher-driven rediscovery.

## Decision

**Skills live behind the same center-store pattern plugins use, but with the wiring collapsed away.** `~/.dsh/skills-store/` holds authoritative copies with `.dsh-source.json` provenance; `<DSH_HOME>/skills/` is the kernel-read user-dsh root that serves as the single materialization target for every installed kernel. Because `dsh-skill-filesystem` already scans that root and watches it with chokidar, materializing a skill is the entire wiring step — no profile edit, no `pnpm install`, no per-kernel copy.

**Install unit is the package, materialization unit is the skill.** An npm tarball / git repo / local folder may contain many skills (monorepo layouts at `skills/<name>/SKILL.md`); the shell scans up to depth 3 and links each skill individually. Enable/disable toggles remove or restore one link. Updates scan upstream, re-link skills whose path moved or whose copy mode is `copy`, unlink skills upstream removed, and keep every surviving skill's previous enable state. Local folders stay out of version checks but keep a manual "重新同步" path that re-runs the same reconcile.

**Hot effect replaces restart.** The kernel's chokidar root watcher plus `skills/change` invalidation means every install / uninstall / enable / disable is visible to live sessions in the next model step — no kernel restart, the opposite of plugins which snapshot profile layers at boot.

**Frontmatter is shell-validated to a top-level subset before install.** The parser reads `name` / `description` at the top level (with quote stripping); candidates the kernel would silently ignore surface as warnings during install so the user knows "上游不会读它" instead of "装了却不出现". A package with zero valid skills fails the install loudly.

**Staging and crash recovery follow the plugin vocabulary.** `.tmp-<pid>-<nanos>` → `.new-<…>` → `.backup-<…>` with `.dsh-id` markers, grouped by id on `reconcile()`. The `.dsh-id` stamp happens AFTER the fetch returns — `git clone` requires an empty destination, `npm` tarball extraction would overwrite the marker, and `copy_tree` removes the dest dir before copying; pre-fetch stamping only ever worked for npm by accident. A startup `reconcile()` also re-links broken store→root entries, removes lingering entries for disabled skills, and sweeps active-root symlinks that point into the store but match no current inventory row. Plain files, directories, and links pointing outside `skills-store/` are user content and never touched.

**v1 ships manual install only — no community catalog card.** The skill panel mirrors the plugin panel's "input + mode select + install button" row and parses the same input shapes (npm spec, git URL with optional `#tag`, owner/repo shorthand, local folder with `local:` prefix / absolute path / `~/…` / Windows drive path). GitHub `dsh-skill` topic is a permanent link in the panel footer so users can browse community resources and paste an address back into the manual install row. A community catalog card like the plugin center would need a stable hub feed for skills, which is not deployed yet; the URL constant and the hub-or-market JSON parser shape are reserved in the design doc so adding the card back later is a localized change.

## Alternatives considered

**Materializing each skill into a separate cordis-managed path or a custom `Config.customSkillDirs` entry.** Rejected: requires a cordis patch layer that the shell explicitly does not write (the user owns `cordis.patch.yml`), and adds a startup reload for every change. The kernel's user-dsh root is already wired and watched.

**Per-kernel materialization (one `plugins/<id>`-style link per kernel version) like plugins.** Rejected: skills do not depend on the kernel's `node_modules`, so per-kernel copies buy nothing and would force every shell command to iterate `kernel::list_installed` — switching kernels becomes unnecessary work for a feature whose whole point is "全局一份".

**A separate `~/.dsh/skills/` directory alongside `~/.dsh/skills-store/` for user-placed content not owned by the shell.** Skipped: the kernel user-dsh root and the shell's materialization target already overlap at `<DSH_HOME>/skills`. Sharing the directory keeps the kernel view single-sourced; the shell's ownership is recorded in `store.json` and the orphan sweep protects user-placed content by checking link targets under `skills-store/` rather than file shape.

**Pre-parsing full YAML via a new dependency.** Rejected: the kernel already validates the full document at load time, so the shell only needs to fail loudly when its preview could not read name/description. A top-level subset parser handles every common frontmatter and keeps the dependency surface flat.

**Auto-running `pnpm install` inside the store to support skills that ship dependencies.** Rejected: skills are instruction data, not packages with runtime deps; the few that reference scripts use `resourceBase` resolution at load time. If a future skill needs transitive packages, the right move is a separate agent-provider registration rather than a per-skill build pipeline in the shell.

## Consequences

- A `skills::SkillStoreItem.skills[].path` is opaque — it points at the package-relative bundle directory or flat file. Renaming a package's internal layout does not break the user-visible frontmatter name, but downstream tooling that walks the store directly must respect it.
- The active-root entry name is the kebab-case frontmatter name, not the bundle directory's name. Two packages can both ship a skill called `pdf` only if the upstream project resolves that collision (the shell refuses duplicates within one package but cannot resolve duplicates across packages — the first installer wins the link).
- Tauri shell has one new `Skill` error variant in `error.rs` mirroring `Plugin`; users see "技能错误：…" rather than a generic "I/O 错误".
- A community catalog card is intentionally out of scope for v1 (no stable hub feed); the design doc reserves the URL constant and JSON shape so reintroducing it is a localized change in `ui/skills.js` + `commands.rs` + `skills.rs`, not a redesign.
- The README user doc adds a single "技能管理" bullet parallel to the plugin one; deeper design lives in `desktop/docs/skill-management.md`, which is updated in the same change to match what shipped.