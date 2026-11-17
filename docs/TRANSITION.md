# MANDATORY — New Machine Transition (read before ANY autonomous work)

> This file is the **first gate** on a freshly relocated checkout. Before reading, writing, testing, delegating, or starting any phase, an agent MUST read this file completely and complete every step below. Do not skip steps because "the code looks fine". A new laptop has **no installed toolchain** and no configured credentials; assumptions from the previous machine are invalid.

## Why this exists

The project was moved to a new Windows machine that did not inherit the old environment. Two classes of failure must be eliminated before work resumes:

1. **Absolute machine-specific paths** committed into tracked files (e.g. `D:\Hermes\Hermes Projects\Manhwa\node_modules`, `C:\Users\<real-user>\...`). These break on a different drive/user and leak machine data. Paths must be relative, env-var based, or derived from `git rev-parse --show-toplevel`.
2. **Missing or mismatched toolchain** (Rust, Tauri, Node/pnpm, Edge for Playwright, `gh`, GitHub PR checks, CodeScene, CodeRabbit). All release gates in `docs/RELEASE_READINESS.md` assume these exist.

## Step 0 — Verify you are on the latest, clean state

From the repository root:

```powershell
git status -sb
git fetch origin
git pull --ff-only
git log -1 --oneline
git remote -v
```

- Working tree must be clean (`## main...origin/main` with no modifications). If dirty, **stop** and report; do not auto-stash.
- Confirm HEAD equals `origin/main`. The relocation must start from the newest commit so no merge conflicts are carried over.
- Confirm `git config user.email` is a `users.noreply.github.com` address and `git config user.name` is machine-neutral. Fix locally before any commit.

## Step 1 — Eliminate absolute machine paths

Run from the repository root and inspect every hit:

```powershell
# Windows drive / real-user paths in tracked files
git grep -n -I -E '(D:|C:)\\\\Users\\\\|D:\\\\Hermes|[A-Za-z]:\\\\Hermes' -- .
# Absolute D:/C: drive roots in text files
git grep -n -I -E '([A-Z]:)[/\\\\](Users|Hermes|Program|Temp)' -- .
```

Rules:
- **Allowed (synthetic fixtures):** `C:\Users\Alice\...`, `C:\Temp\...`, `C:\Cache\...` used only in tests as fake paths — but verify they are clearly fictional and never a real username.
- **Forbidden:** any path containing the previous machine's user name, `D:\Hermes\Hermes Projects\...`, or an absolute drive-root that is not obviously a synthetic fixture.
- **Fix pattern:** replace with a relative path, `%LOCALAPPDATA%`, `$env:TEMP`, or compute the repo root with `(git rev-parse --show-toplevel)`. Never hardcode the checkout location.
- Check at minimum: `docs/**`, `AGENTS.md`, `scripts/**`, `.github/workflows/**`, `app/frontend/**` (including `playwright.config.ts`, `vite.config.ts`, `tsconfig*`, `package.json`, `e2e/**`), `app/backend/**`, `Cargo.toml`/`.cargo/**`, `.vscode/**`, `.agents/**`, `GLOSSARY.md`, `README*`.
- If any real machine path is found, **delegate the fix to a sub-agent** (the Orchestrator never edits implementation code directly), run the relevant gates, and publish via the normal PR flow. Do not proceed to feature work with known machine paths remaining.

## Step 2 — Bootstrap and verify the environment

Run the repository script (safe check mode first):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/bootstrap-new-machine.ps1
```

It verifies and reports every tool below. On a fresh machine, run with `-Install` (as the script's own output instructs) to install the missing pieces that can be installed non-interactively, then re-run check mode until everything is `OK`.

Required tooling and the exact checks:

| Tool | Check | Notes |
|---|---|---|
| Git + GitHub CLI | `git --version`, `gh --version`, `gh auth status` | `gh auth login` if not authenticated; PR/issue workflow depends on it |
| Rust toolchain | `rustc --version`, `cargo --version` | CI pins Rust `1.99.1`; use `rustup` to match. Add `rustfmt`, `clippy` |
| Cargo target hygiene | `$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) "target")` | All builds must land here, never in per-worktree folders |
| Node + pnpm | `node --version`, `pnpm --version` | `corepack enable` if pnpm is absent; CI uses pnpm 9 / Node 20+ |
| Tauri CLI / WebView2 / NSIS | `cargo tauri --version` | Needed for `scripts/package-windows.ps1` default mode; `-StandaloneOnly` is not CI-equivalent |
| Microsoft Edge (system channel) | Edge present; Playwright config uses `channel: "msedge"` | Do **not** download Playwright browsers |
| CodeRabbit | GitHub App enabled on the repo; PR checks visible | **Never run the `coderabbit` CLI locally** — this is a permanent AGENTS.md rule. Verification is purely via GitHub PR checks/reviews; trigger with `@coderabbitai review` on OSS PRs when skipped |
| CodeScene | PR check `CodeScene Code Health Review (main)` visible on GitHub; MCP tools if available in-session | Verify the check exists on a PR before merging |
| GitHub Actions CI | `gh pr checks <n>` shows `gate-a-and-backend`, `gate-g-frontend`, `mutation-gate`, `package-windows` | All must be green before merge |

The script prints `MISSING` with an exact remediation command for each failed check. Do not guess; do not continue with a partially prepared environment.

## Step 3 — Prove the environment works before resuming work

```powershell
# From repo root, with the repo-root cargo target dir set
$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) "target")
powershell -ExecutionPolicy Bypass -File scripts/gate-a-check.ps1
cargo fmt --manifest-path app/Cargo.toml --all -- --check
pnpm --dir app/frontend run typecheck
pnpm --dir app/frontend run test
pnpm --dir app/frontend run test:e2e      # msedge, no browser download
```

Record the results. A failure here means the environment is not ready — fix it before touching product code.

## Step 4 — Only now resume autonomous work

Once Steps 0–3 pass, tell the owner the environment is verified and resume the normal `$to-spec`/`$to-tickets`/`$implement` flow and the PR merge gate from `AGENTS.md`. Until then, your only job is this transition.

## Non-negotiable

- No live deletion on real `C:` user/system paths outside the sentinel sandbox — ever, without an explicit owner action.
- No pushing directly to `main`; every change goes through a PR with CI + CodeScene + CodeRabbit green and zero unresolved review threads.
- No personal emails, real usernames, absolute machine paths, session IDs, or secrets in tracked files.
