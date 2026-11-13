# Sweeper

Safety-first Windows desktop disk cleaner (Tauri 2 + Rust + React/TypeScript) that discovers reclaimable space on `C:`, explains every item's purpose, size, and `Risk`, and permanently removes only what the user confirms.

## Fixed Principles

- **Safety over features and speed:** Deny-by-default (`Allowlist` + validated `Custom Path`), non-disableable `Protected Path`s, handle-based removal with moment-of-deletion re-validation, `Link Point`s never followed, cloud placeholders always `Skipped`, and gated releases (`proptest`, `cargo-fuzz`, `cargo-mutants` ≥ 95%, sandbox TOCTOU/canary tests).
- **Three separated parts:** `backend/` (pure Rust safety & NTFS logic; `unsafe` confined to `sweeper-winfs`), `bridge/` (`sweeper-contract` single source of truth + `sweeper-desktop` Tauri shell; ID-only deletion IPC), and `frontend/` (React/TS UI only; zero `fs`/`shell` plugins).
- **Autonomous execution:** The owner has granted full standing approval for all phases (0–6). Drive `$to-spec` → `$to-tickets` → `$implement-spec` → `$code-review` → `$retro` continuously via sub-agents without pausing for permission. Live deletion on real `C:` user/system paths outside the sentinel test sandbox remains strictly forbidden until manual release verification.
- **Build & artifact hygiene:** Direct all Cargo builds into `<repo-root>\target` (`CARGO_TARGET_DIR`) instead of per-worktree `target/` folders on `C:`, use existing system Edge (`channel: "msedge"`) for Playwright instead of downloading browsers, keep GitHub Actions CI disabled locally, and run `scripts/cleanup-artifacts.ps1` after every closed ticket and via the periodic watchdog (strictly inside `<repo-root>`, never touching `C:` or `.t3`).

## Context Pointers

- **Orchestration, thread authority, and sub-agent model:** Read [docs/DELEGATION.md](docs/DELEGATION.md) before reading, writing, testing, or reviewing any code, or spawning any sub-agent.
- **Architecture, safety invariants, phases (0–6), IPC contract, and gates (A–H):** Read [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) before designing, planning, or implementing any module or feature.
- **Domain terminology and architectural rationale:** Consult [GLOSSARY.md](GLOSSARY.md) for canonical terms (`Target`, `Allowlist`, `Protected Path`, `Scan`, `Preview`, `Clean`, `Analyzer`, `Risk`, `Skipped`, `Link Point`) and [docs/adr/](docs/adr/) (`0001`–`0003`) before introducing types, UI copy, or structural changes.

## Agent skills

### Issue tracker

GitHub Issues on `mohamedix0/Sweeper` via `gh` CLI. See [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).

### Triage labels

Default five-label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See [docs/agents/triage-labels.md](docs/agents/triage-labels.md).

### Domain docs

Single-context (`GLOSSARY.md` at repo root and `docs/adr/`). See [docs/agents/domain.md](docs/agents/domain.md).
