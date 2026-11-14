# Sweeper — Release Readiness Runbook

> Scope: Phase 6 / Gate H release handoff for `v0.1.0`-line Windows builds of Sweeper.
> Canonical terms: [../GLOSSARY.md](../GLOSSARY.md). Architecture and gates: [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md) (§6.2 Gates A–H, Phase 6). Governance: [DELEGATION.md](./DELEGATION.md), [../AGENTS.md](../AGENTS.md).

This runbook separates **automated release evidence** (Gates A–G, mutation scoring, and packaging/checksum verification) from **human-only release actions** (Gates H.3, H.4, and H.5). It is written to be executed from a clean clone/worktree of `mohamedix0/Sweeper` on Windows using repository scripts and workflows as the source of truth.

## 1. Boundary Policy — Why Automation Stops Here

Sweeper's safety model is deny-by-default and defense-in-depth (validated `Allowlist` / `Custom Path`s, non-disableable `Protected Path`s, handle-based moment-of-deletion re-validation, `Link Point`s never followed, cloud placeholders always `Skipped`, ID-only deletion IPC, and bounded `ScanSession` / `preview_token` lifetimes). Automated tests and CI **must not** perform live deletion on real user/system paths.

Automated checks therefore stop at **sentinel sandboxes / isolated temp fixtures** (`sweeper-testkit` canary + sentinel harness, `cargo test` integration suites, fuzz/mutation targets) and **mock-backend frontend/E2E runs** (Vitest + Playwright against the Vite dev server with `VITE_SWEEPER_BACKEND=mock`, as configured in `app/frontend/playwright.config.ts`).

The following remain **human-gated** on a real Windows machine because they require operator judgment, real filesystem state, and/or release credentials that must never enter the repository or CI logs:
- **Gate H.3:** Real-machine read-only `Scan` inspection (including `Risk`, byte sizes, `Protected`/`Unknown` behavior, and elevation-gated target visibility) before any real `Clean`.
- **Gate H.4:** First real `Clean` limited to `user-temp` only, followed by log inspection in `%LOCALAPPDATA%\Sweeper\logs\`.
- **Gate H.5:** Production Authenticode signing and final checksum/signature verification of distributable artifacts (no signing keys, certificates, passwords, or thumbprints are stored in the repo).

> **Non-negotiable:** Do not run real deletion outside the sentinel test sandbox on real `C:` data as part of automated release steps. `Scan` is read-only; `Preview`/`Clean` on real data are manual, owner-confirmed actions only.

## 2. Automated Release Gates (H.1 & H.2) — Pre-Release Evidence Checklist

Use a repo-root `CARGO_TARGET_DIR` (for example `$env:CARGO_TARGET_DIR = "<repo-root>\target"` or `Join-Path (git rev-parse --show-toplevel) "target"`) so transient Rust outputs stay inside the repository root. Run repository hygiene after local verification: `powershell -ExecutionPolicy Bypass -File scripts/cleanup-artifacts.ps1`.

### Gate A — Static / Privacy / Dependency Gates (`gate-a-and-backend`)
CI job: `.github/workflows/ci.yml` → `gate-a-and-backend` (Windows, Rust `1.99.1`, `CARGO_TARGET_DIR=${{ github.workspace }}/app/target`).

| Check | Command / source of truth |
|---|---|
| Rust formatting | `cargo fmt --manifest-path app/Cargo.toml --all -- --check` |
| Gate A static & privacy checks | `powershell -ExecutionPolicy Bypass -File scripts/gate-a-check.ps1` (bans deletion primitives outside allowed modules, enforces bridge dependency/privacy-scrub rules, fails on forbidden local paths/PII patterns in tracked files) |
| Clippy (workspace, deny warnings) | `cargo clippy --manifest-path app/Cargo.toml --workspace --all-targets --locked -- -D warnings` |
| Rust dependency/license/advisory checks | `cargo deny check` (from `app/`) and `cargo audit` (from `app/`) |
| Frontend dependency audit (high) | `pnpm --dir app/frontend audit --audit-level=high` (run in `gate-g-frontend` in CI) |
| Frontend type/lint gates | `pnpm --dir app/frontend run typecheck` and `pnpm --dir app/frontend run lint` (run in `gate-g-frontend` in CI) |

Local preflight minimum (before broader test runs): `scripts/gate-a-check.ps1`, `cargo fmt --check`, and `cargo clippy` as above.

### Gates B–F — Backend / Bridge Safety & Correctness (`gate-a-and-backend`)
CI runs: `cargo test --manifest-path app/Cargo.toml --workspace --locked`.

This workspace test run is the evidence bucket for Gates B–F (unit/property/adversarial/path/NTFS/remover/allowlist/bridge/settings/elevation coverage as implemented in `app/backend/` and `app/bridge/`). Representative suites present in the workspace include (non-exhaustive): `gate_b_proptest`, `gate_c_adversarial`, `gate_c_ntfs`, `gate_d_remover`, `gate_e_allowlist`, `gate_f_bridge`, `settings_tier3`, `elevation_targets`, `allowlist_lint`, `read_only`, and analyzer tests. Treat **all** workspace tests passing under `--locked` as required.

### Gate G — Frontend / E2E against Mock Backend (`gate-g-frontend`)
CI job: `.github/workflows/ci.yml` → `gate-g-frontend` (Windows, Node 20, pnpm 9, `CI=true`).

| Step | Command (from repo root unless noted) |
|---|---|
| Install | `pnpm --dir app/frontend install --frozen-lockfile` |
| Dependency audit | `pnpm --dir app/frontend audit --audit-level=high` |
| Typecheck | `pnpm --dir app/frontend run typecheck` |
| Lint | `pnpm --dir app/frontend run lint` |
| Unit/component tests (Vitest) | `pnpm --dir app/frontend run test` (`vitest run`) |
| Production build | `pnpm --dir app/frontend run build` (`tsc --noEmit && vite build`) |
| E2E (Playwright) | `pnpm --dir app/frontend run test:e2e` (`playwright test`) |

Playwright is configured in `app/frontend/playwright.config.ts` to use the installed Microsoft Edge channel (`channel: "msedge"`), headless mode, base URL `http://127.0.0.1:4173`, and a Vite dev server command that forces mock backend mode (`VITE_SWEEPER_BACKEND=mock`). CI must not download Playwright browsers.

### Mutation Gate — H.2 (`mutation-gate`)
CI job: `.github/workflows/ci.yml` → `mutation-gate` (Windows, Rust `1.99.1`, `CARGO_TARGET_DIR=${{ github.workspace }}/app/target`).
Script: `scripts/run-cargo-mutants.ps1` (targets `sweeper-core` files `backend/sweeper-core/src/policy.rs` and `backend/sweeper-core/src/paths.rs`; enforces a minimum mutation score of **95%** by default).

| Purpose | Command |
|---|---|
| Script/parser/threshold self-test (required in CI and before local mutation runs) | `powershell -ExecutionPolicy Bypass -File scripts/run-cargo-mutants.ps1 -SelfTest` |
| Run mutation gate | `powershell -ExecutionPolicy Bypass -File scripts/run-cargo-mutants.ps1` |
| Optional arguments (as implemented) | `-MinScore <double>` (default `95.0`), `-OutputDir <path>` (default `<repo-root>\target\mutants`), `-ListOnly`, `-SelfTest` |

Artifacts: `<OutputDir>\mutation-summary.json`, `<OutputDir>\mutants.out\outcomes.json`, and `<OutputDir>\mutants.out\missed.txt` (when present). A failing self-test or score below threshold is a release blocker.

### Packaging & SHA-256 Gate (`package-windows`)
CI job: `.github/workflows/ci.yml` → `package-windows` (Windows, Rust `1.99.1`, Node 20, pnpm 9, Tauri CLI installed, NSIS ensured in CI).
Script: `scripts/package-windows.ps1` (builds frontend, `cargo build --manifest-path app/Cargo.toml --package sweeper-desktop --release --locked`, and by default a Tauri NSIS bundle; then copies artifacts, optionally signs, and generates/verifies `SHA256SUMS.txt`).

| Mode | Command | Expected outputs (in `-OutputDir`, default `<repo-root>\target\release-artifacts\windows`) |
|---|---|---|
| Default (standalone + NSIS installer) | `powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1` | `Sweeper-<version>-windows-x64-standalone.exe`, `Sweeper-<version>-windows-x64-nsis-setup.exe`, `SHA256SUMS.txt` (sorted, LF, UTF-8 without BOM; verified by script) |
| Standalone-only (local/dev environments without Tauri CLI/NSIS) | `powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1 -StandaloneOnly` | `Sweeper-<version>-windows-x64-standalone.exe`, `SHA256SUMS.txt` (no NSIS installer) |

Supported script flags (as implemented): `-OutputDir <path>`, `-SkipFrontendInstall` (requires existing `app/frontend/dist/index.html`), `-StandaloneOnly`.
CI requires **both** `.exe` artifacts plus `SHA256SUMS.txt` and uploads `target/release-artifacts/windows/*.exe` and `target/release-artifacts/windows/SHA256SUMS.txt`.

> **Honesty note:** Local packaging evidence is only equivalent to CI's packaging gate when run in **default mode** on a machine with Tauri CLI and NSIS available and producing/verifying both installer and standalone artifacts. `-StandaloneOnly` is acceptable for local standalone verification but does **not** replace the CI/NSIS packaging evidence.

## 3. Human-Only Gate H.3 — Real-Machine Read-Only `Scan` Review

**Goal:** Confirm Sweeper's discovery/classification behavior on the actual target machine before any `Clean`, with zero deletion side effects.

**Prerequisites:** Gates A–G, mutation gate, and packaging evidence are green for the candidate commit. Use a build produced from that commit. Ensure logs/settings paths resolve to the standard locations for the running user (`%APPDATA%\Sweeper\settings.json` for settings, `%LOCALAPPDATA%\Sweeper\logs\` for operation logs) and that no untrusted/tampered settings bypass review.

### Checklist (owner/operator)
- [ ] **Run as standard user first (default, non-elevated):** Launch Sweeper without elevation and perform a read-only `Scan` (default tiers as configured). Confirm the UI/preview data reflects `Scan` results only (no `Clean`).
- [ ] **Inspect discovered items:** Review each listed `Target`/item for expected identity, description, `Risk` badge (`Safe` / `Caution` / `Unknown` / `Protected` where surfaced), and reported byte counts/sizes. Spot-check obviously incorrect sizes or unexpected paths before proceeding.
- [ ] **Verify protection behavior:** Confirm `Protected Path`s and non-selectable/protected items are not presented as cleanable. Confirm `Unknown` items are not selectable unless explicitly allowed by settings and gated flows (not part of first-clean). Confirm any items that should be `Skipped` by policy are not misclassified as cleanable.
- [ ] **Review Tier 3 / elevation-gated targets (read-only):** If evaluating Phase 6 elevation behavior, run an **elevated** read-only `Scan`/inspection and confirm only the audited elevation-requiring `Target`s are unlocked for consideration when elevation is enabled: `windows-temp`, `windows-update-download-cache`, `windows-prefetch`, and `programdata-package-cache` (all `Tier3`, `Caution`, `requires_elevation=true` in `app/backend/sweeper-knowledge/allowlist.toml`). Confirm they display `"Requires administrator"` / elevation gating in the UI where applicable and that no other protections are weakened.
- [ ] **No unexpected selection state:** Ensure nothing outside the intended reviewed set is preselected and that no `Custom Path` or `Unknown` selection is included for the first live clean.
- [ ] **Stop condition:** If any classification, size, path, or gating looks wrong, **do not proceed to H.4**. Capture evidence (screenshots/log IDs if applicable) and investigate before any real `Clean`.

## 4. Human-Only Gate H.4 — First Real `Clean` (`user-temp` Only) + Log Inspection

**Goal:** Perform the first real deletion on the target machine with minimum blast radius and verify auditable outcomes.

**Scope constraint:** The first real `Clean` must include **only** the `user-temp` `Target` (`Tier1`, `Safe`, contents-only; `FOLDERID_LocalAppData\Temp` per `app/backend/sweeper-knowledge/allowlist.toml`). Do **not** include `Tier2`/`Tier3`, `Custom Path`s, `Unknown` items, or elevation-gated targets in this first run.

### Checklist (owner/operator)
- [ ] **Fresh read-only `Scan`:** Run a new `Scan` on the target machine and review results (H.3 baseline still applies).
- [ ] **`Preview` review:** Open `Preview` and confirm the breakdown contains **only** `user-temp`. Verify estimated bytes/counts are plausible and no other `Target` is included.
- [ ] **Select only `user-temp`:** Ensure the selection is limited to `user-temp` (`Safe`, `Tier1`). Deselect everything else. Do not provide confirmations for `Custom Path` / `Unknown` flows.
- [ ] **Execute `Clean`:** Run `Clean` (which must follow the standard `Preview` → `execute_clean(preview_token)` flow) and wait for completion summary.
- [ ] **Inspect operation logs:** Review JSON-lines logs under `%LOCALAPPDATA%\Sweeper\logs\` (resolved by `sweeper-core` journal logic; default base `%LOCALAPPDATA%\Sweeper`, `logs` subdirectory). Confirm records show expected intent/outcome entries for the `user-temp` run, including paths, sizes/bytes, timestamps, statuses (`deleted` / `Skipped`), and reasons for any `Skipped` items.
- [ ] **Verify outcomes:** Confirm reported bytes freed and item outcomes match expectations for temp-content cleanup. Confirm no unexpected paths/targets were acted on and no unintended deletions occurred.
- [ ] **If anything is unexpected:** Stop further real cleans, preserve logs, and investigate before expanding scope.

## 5. Human / Release Gate H.5 — Authenticode Signing & Final Checksum Verification

**Goal:** Produce (or validate) signed release artifacts and verify integrity metadata before distribution.

### Signing (non-secret instructions)
`scripts/package-windows.ps1` signs `.exe` artifacts **only when signing credentials are configured** via environment variables (no secrets in repo/CI config):
- `SWEEPER_CERT_PFX_PATH` — path to a `.pfx` certificate file (used with `signtool sign /f ...`).
- `SWEEPER_CERT_PASSWORD` — optional PFX password (used with `/p ...` when `SWEEPER_CERT_PFX_PATH` is set).
- `SWEEPER_CERT_THUMBPRINT` — certificate thumbprint in the certificate store (used with `/sha1 ...`).
- `SWEEPER_TIMESTAMP_URL` — optional RFC 3161 timestamp server URL (used with `/tr ... /td SHA256`).

When signing is enabled, the script locates `signtool.exe`, signs each `.exe` with SHA-256 file digest, and runs `signtool verify /pa` per artifact. If neither `SWEEPER_CERT_PFX_PATH` nor `SWEEPER_CERT_THUMBPRINT` is set, the script reports that signing is skipped and continues with unsigned artifacts (not sufficient for a signed release).

**Release procedure (human/operator):**
1. Build/package the candidate commit in a clean environment (default mode for standalone + NSIS): `powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1`
2. Provide signing credentials **out-of-band** (CI secrets, secure local secret store, or operator-provided ephemeral env vars). Never commit certificates, passwords, or thumbprints.
3. Confirm script output shows successful `signtool sign` and `signtool verify /pa` for every `.exe` in the output directory.
4. Record the signing identity/certificate metadata in the release record outside the repository (release ticket/notes), not in tracked files.

### Verify signatures
For each distributable `.exe` (standalone and NSIS installer, as applicable):

```powershell
signtool verify /pa "<artifact.exe>"
```

Optionally inspect details in Windows Explorer (Digital Signatures tab) or with `Get-AuthenticodeSignature "<artifact.exe>"` and confirm `Status` is valid/trusted for the intended certificate.

### Verify `SHA256SUMS.txt`
`scripts/package-windows.ps1` generates `SHA256SUMS.txt` (sorted entries, LF line endings, UTF-8 without BOM, format `<sha256>  <filename>`) and verifies it before exit. Before publishing, re-verify independently:

```powershell
# From the artifact directory containing *.exe and SHA256SUMS.txt
$expected = @{}
Get-Content -LiteralPath .\SHA256SUMS.txt | ForEach-Object {
  if ($_ -match '^([0-9a-f]{64})  (.+)$') { $expected[$Matches[2]] = $Matches[1] }
}
Get-ChildItem -LiteralPath . -File -Filter *.exe | Sort-Object Name | ForEach-Object {
  $actual = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($expected[$_.Name] -ne $actual) { throw "Checksum mismatch: $($_.Name)" }
  "OK  $($_.Name)"
}
```

Confirm `SHA256SUMS.txt` contains exactly the intended distributable `.exe` files for the release (CI default: standalone + NSIS installer). Do not publish artifacts whose signatures or checksums do not verify.

## 6. Release Evidence Record (Recommended)

For the candidate commit/tag, capture (outside tracked PII/secrets): commit SHA, CI run links/results for `gate-a-and-backend`, `gate-g-frontend`, `mutation-gate`, and `package-windows`, local verification command transcript summary (if run), mutation score and artifact locations, packaged artifact names + SHA-256 hashes, H.3/H.4 completion notes and log review outcome, and H.5 signing verification evidence. This record supports reproducible go/no-go decisions without weakening safety gates.
