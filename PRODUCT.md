# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Windows desktop users and developers reclaiming disk space on `C:` who need complete transparency into what will be removed, why each item exists, its `Risk` classification, and rock-solid protection against deleting user data, repositories, credentials, or OS state.

## Product Purpose

**Sweeper** is a safety-first Windows desktop disk cleaner (Tauri 2 + Rust + React/TypeScript) that discovers reclaimable space across `Allowlist` `Target`s and validated `Custom Path`s, explains every item's purpose, size, and `Risk`, previews exact byte breakdowns before acting, and permanently removes only what the user explicitly confirms.

## Positioning

Unlike opaque one-click disk cleaners, Sweeper is **deny-by-default**: non-disableable `Protected Path`s, handle-based NTFS removal with moment-of-deletion re-validation, `Link Point`s never followed, cloud placeholders always `Skipped`, ID-only IPC with no frontend filesystem access, and auditable JSON-lines operation logs.

## Operating Context

- Desktop utility window (`Sweeper`, WebView2 inside Tauri 2) running on Windows as standard non-elevated user by default, with optional explicit **Run as administrator** relaunch strictly for elevation-gated `Tier` 3 `Target`s.
- Four primary task surfaces: **Scan & Preview**, **Analyzer**, **Settings**, and **Logs**, plus an **Explain** detail drawer.
- Strict Content Security Policy (`default-src 'self'; style-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost`).

## Capabilities and Constraints

- **Canonical domain terms:** `Target`, `Allowlist`, `Protected Path`, `Scan`, `Preview`, `Clean`, `Analyzer`, `Risk` (`Safe`, `Caution`, `Unknown`, `Protected`), `Skipped`, `Link Point`, `Tier` 1/2/3, `Custom Path`, `Minimum Age`.
- **Two-step destructive safety:** `Scan` (read-only) → `Preview` (`preview_token`, 10-minute TTL) → explicit `Clean Permanently` (with typed `DELETE` confirmation for `Custom Path` / `Unknown` items and 120% byte circuit breaker).
- **Display-only protections:** WSL/Docker `.vhdx` files, `Protected Path`s, and `Link Point`s are visible in `Analyzer` as non-selectable/disabled.
- **Technical constraints:** Zero external font/asset CDNs, zero inline `<style>` tags, zero static inline `style=` violations under CSP, system/local typography only, and all accessible names/roles/`aria-label`s preserved for automated tests.

## Brand Commitments

- **Product name:** Sweeper
- **App icon:** Obsidian rounded-squircle tile with faceted cobalt-to-teal protective shield and diagonal ice-white sweep (`app/frontend/src/assets/sweeper-icon.png`).
- **Owner aesthetic directive (explicit brief):** Modern, sleek glassmorphic ("عصري وزجاجي") desktop utility aesthetic without compromising scanability, contrast, or safety clarity.

## Evidence on Hand

- Live Rust/Tauri desktop application (`app/bridge/sweeper-desktop/`) and React/TypeScript frontend (`app/frontend/`).
- Mock-mode Playwright fixtures and real WebView2 CDP E2E (`app/frontend/scripts/test-desktop-e2e.mjs`).
- App icon assets in `app/bridge/sweeper-desktop/icons/` and `app/frontend/src/assets/sweeper-icon.png`.

## Product Principles

1. **Safety visibility over visual ornament:** `Risk` states, `Protected` guards, elevation locks, and `DELETE` confirmations must read unmistakably at a glance.
2. **Operational precision:** Tabular numbers (`tabular-nums`) for all byte counts, paths in monospace, and dense scanability across large trees and breakdowns.
3. **Calm before irreversible action:** Destructive permanent deletion (`Clean Permanently`) must never look like a routine safe action (`Start Scan` / `Preview Clean`).
4. **Strict CSP & native performance:** Crisp translucent glass surfaces achieved cleanly via local CSS custom properties, `backdrop-filter`, hairline specular borders, and composite-only motion (`150–250ms`).
