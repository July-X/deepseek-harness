#!/usr/bin/env node
// Detect-then-install wrapper.
//
// desktop is intentionally a standalone deliverable that does NOT join the
// repository's pnpm workspace — passing `--ignore-workspace` keeps
// `pnpm install` rooted at this directory instead of hoisting the whole
// monorepo. When pnpm is missing we fall back to npm so users without pnpm
// can still pull in `@tauri-apps/cli` and run `tauri dev` / `tauri build`.
//
// Invoke through `npm run deps` or `pnpm run deps` from the desktop
// directory; never call this file directly (it lives under scripts/ and is
// wired into package.json's `scripts.deps`).

import { execFileSync } from 'node:child_process';

const isWin = process.platform === 'win32';
// `.cmd` shims cannot be spawned directly on Windows (Node returns EINVAL);
// the package manager scripts in the desktop project route through
// `%ComSpec% /C` for the same reason.
const comspec = isWin ? (process.env.ComSpec || 'cmd.exe') : null;

function run(cmd, args) {
  if (isWin) {
    return execFileSync(comspec, ['/C', cmd, ...args]);
  }
  return execFileSync(cmd, args);
}

function has(cmd) {
  try {
    run(cmd, ['--version']);
    return true;
  } catch {
    return false;
  }
}

const usePnpm = has('pnpm');
const pkgMgr = usePnpm ? 'pnpm' : 'npm';
// `--ignore-workspace` is pnpm-specific; npm has no concept of the
// repository root's pnpm-workspace.yaml and does not need the flag.
const args = usePnpm ? ['install', '--ignore-workspace'] : ['install'];

if (!usePnpm) {
  console.log('[install] pnpm 未检测到，回退到 npm');
}
console.log(`[install] 正在执行：${pkgMgr} ${args.join(' ')}`);

try {
  run(pkgMgr, args);
} catch (err) {
  // stdio: 'inherit' is set on the wrapper above so the child output has
  // already streamed to the terminal; only the failure status is ours to
  // surface here.
  console.error(`[install] ${pkgMgr} install 失败（退出码 ${err.status ?? '?'}）`);
  process.exit(err.status ?? 1);
}