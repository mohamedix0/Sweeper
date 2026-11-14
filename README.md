# Sweeper

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![CI](https://github.com/mohamedix0/Sweeper/actions/workflows/ci.yml/badge.svg)](https://github.com/mohamedix0/Sweeper/actions/workflows/ci.yml)

A safety-first Windows desktop disk cleaner built with **Rust**, **Tauri 2**, and **React / TypeScript**.

Sweeper discovers reclaimable disk space on `C:`, explains every item's purpose, size, and `Risk`, and permanently removes only what the user explicitly inspects and confirms.

> **Status:** Active development toward v1.0. Phases 0–4 (Foundation, Read-Only Core, `Scan` / `Preview` UI, Handle-Based `Remover` + `Clean`, and Read-Only Drive `Analyzer`) are complete; Phase 5 (Advanced Settings, `Custom Path`s, and `Tier` 3 `Target`s) and Phase 6 (Windows packaging and optional Admin elevation) are in progress. See [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md).
> **Release handoff:** Automated gates, packaging evidence, and human-only release steps are documented in [docs/RELEASE_READINESS.md](docs/RELEASE_READINESS.md).

---

## Core Safety Principles

- **Deny-by-default (`Allowlist`):** Only curated `Target`s in `sweeper-knowledge` (and explicitly validated `Custom Path`s in Phase 5) can ever become cleanable.
- **Non-disableable `Protected Path`s:** Windows OS directories, user documents/desktop/downloads, profile roots, SSH/GPG/keystores, browser profile roots, and active workspace/app roots are unconditionally excluded.
- **Three-part architectural separation:**
  - `app/backend/` — pure Rust NTFS & safety logic (`unsafe` strictly confined to `sweeper-winfs`).
  - `app/bridge/` — typed IPC contract (`sweeper-contract` via `tauri-specta`) and Tauri shell (`sweeper-desktop`, zero `sweeper-winfs` dependency, ID-only deletion commands).
  - `app/frontend/` — React / TypeScript UI only (zero filesystem or shell plugins).
- **Handle-based moment-of-deletion validation:** `sweeper-winfs` opens directory handles with `FILE_FLAG_OPEN_REPARSE_POINT`, re-resolves canonical paths via `GetFinalPathNameByHandleW`, rejects `Link Point`s (junctions/symlinks), skips cloud placeholders, enforces `Minimum Age`, and deletes via `FileDispositionInfoEx`.
- **Two-phase confirmation & circuit breaker:** `Preview` issues a single-use, time-bounded `preview_token` (`10` minutes, bounded by a `15`-minute `ScanSession` TTL), and `Clean` aborts automatically if actual deleted bytes exceed `120%` of the `Preview` estimate.
- **Read-only Drive `Analyzer`:** Fast parallel `FindFirstFileExW` tree walk annotates space with `Knowledge Base` entries and `Risk` badges, routing any cleanable selection strictly through the standard `Preview -> Clean` pipeline (`.vhdx` disk images remain display-only).

---

## Repository Layout

| Path | Purpose |
|---|---|
| [app/backend/](app/backend/) | `sweeper-winfs`, `sweeper-knowledge`, `sweeper-core`, `sweeper-testkit` |
| [app/bridge/](app/bridge/) | `sweeper-contract` (shared DTOs + TS bindings) and `sweeper-desktop` (Tauri 2 shell) |
| [app/frontend/](app/frontend/) | React 18 + Vite + TypeScript UI and Vitest / Playwright suites |
| [app/fuzz/](app/fuzz/) | Fuzz targets (`cargo-fuzz`) for untrusted path and contract inputs |
| [docs/](docs/) | [Implementation Plan](docs/IMPLEMENTATION_PLAN.md), [Delegation Policy](docs/DELEGATION.md), and [ADRs](docs/adr/) |
| [scripts/](scripts/) | Gate A static/privacy checks and repository hygiene scripts |
| [GLOSSARY.md](GLOSSARY.md) | Canonical domain terminology (`Target`, `Allowlist`, `Protected Path`, `Risk`, `Skipped`, `Link Point`, etc.) |
| [AGENTS.md](AGENTS.md) | Repository governance and contributor/agent rules |
| [LICENSE](LICENSE) | Apache License 2.0 |

---

## Development & Verification (Windows)

### Prerequisites

- Windows 10 / 11 (x64)
- Rust toolchain pinned in [app/rust-toolchain.toml](app/rust-toolchain.toml) (`1.99.1`, with `rustfmt` and `clippy`)
- Node.js `20+` and `pnpm` `9+`

### Run Static & Privacy Gates

```powershell
powershell -ExecutionPolicy Bypass -File scripts/gate-a-check.ps1
```

### Run Backend & Bridge Checks

```powershell
cargo fmt --manifest-path app/Cargo.toml --all -- --check
cargo clippy --manifest-path app/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path app/Cargo.toml --workspace --locked
```

### Run Frontend Checks

```powershell
pnpm --dir app/frontend install --frozen-lockfile
pnpm --dir app/frontend run typecheck
pnpm --dir app/frontend run lint
pnpm --dir app/frontend test
pnpm --dir app/frontend run build
```

---

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
