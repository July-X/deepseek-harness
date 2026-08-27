# Agent Note: Keep the Desktop Shell in Its Standalone Repository

Status: implemented

English | [中文](2026-08-28-standalone-desktop-repository.zh.md)

## Problem

The desktop shell has its own Tauri source tree, frontend, release workflow, and runtime dependency policy. Keeping it in this plugin harness repository creates a second ownership path and makes root checks account for a product with a separate release repository.

## Decision

The desktop shell is maintained exclusively in [July-X/dsh-xlink](https://github.com/July-X/dsh-xlink). This repository contains no desktop source tree, desktop release workflow, or desktop-specific CI assertion. The standalone repository owns the Tauri sources, management UI, assets, documentation, tests, and release configuration.

## Alternatives considered

**Keep a synchronized copy in both repositories.** Two copies require synchronization and allow the source trees to diverge, so this option leaves ownership ambiguous.

**Keep a forwarding wrapper or release workflow here.** A wrapper or workflow would retain desktop-specific ownership in this repository and would still make root CI depend on the standalone application's files, so this option does not establish a single owner.

## Consequences

Desktop changes and release checks are made in the standalone repository. The plugin harness repository's root CI and documentation cover the packages and applications that remain here. The archived [desktop shell implementation record](../../archived/feature/2026-08-21-tauri-desktop-shell.md) preserves the implementation rationale without describing current source ownership.
