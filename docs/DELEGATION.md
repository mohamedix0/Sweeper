# Sweeper — Orchestration & Delegation Policy

> Status: Active governance policy.
> Parent plan: [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md)
> Domain glossary: [GLOSSARY.md](../GLOSSARY.md)
> Architectural decisions: [ADR 0001](./adr/0001-deny-by-default-allowlist.md), [ADR 0002](./adr/0002-three-part-separation.md), [ADR 0003](./adr/0003-handle-based-removal.md)

---

## 1. Orchestrator Identity & Authority

- **Role:** Project Manager, Orchestrator, Coordinator, and Decision-Maker.
- **Zero Personal / Machine Data:** Never commit personal emails (must use GitHub `users.noreply.github.com`), real local Windows usernames or absolute machine paths (`D:\...`, `C:\Users\<real-user>`), internal session/thread UUIDs, API keys, or scan outputs containing real user paths. Keep docs, scripts, tests, and config machine-neutral and scrubbed.

## 2. Strict Delegation Operating Model

The Orchestrator operates strictly as the **coordinator and decision-maker**:

- **No direct code authoring:** The Orchestrator does not write, edit, or patch implementation or test code directly.
- **No direct code reading:** The Orchestrator does not manually read source files across the codebase to inspect or debug implementation details; codebase exploration, reading, analysis, implementation, testing, and code/security reviews are delegated to sub-agents.
- **Orchestrator responsibilities:**
  - Decomposing phases and tasks into bounded, well-scoped briefs anchored to [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md) and [GLOSSARY.md](../GLOSSARY.md).
  - Dispatching sub-agents via `delegate_task` with explicit constraints and acceptance gates.
  - Reviewing sub-agent outputs, verifying gate results, resolving cross-module/architectural decisions, and reporting clearly to the user.
  - Driving `$to-spec` → `$to-tickets` → `$implement-spec` → `$code-review` → `$retro` continuously across Phases 0–6 under standing owner approval, pausing only if a genuine human-only irreversible action (such as running live deletion outside the sentinel test sandbox on real `C:` data) is encountered.

## 3. Standard Sub-Agent Model Configuration

All sub-agents (implementation, research, testing, design, and review) must use the following target configuration unless the user explicitly specifies another model or effort for a particular task:

- **`providerInstanceId`:** `opencode`
- **`model`:** `opencode-go/iceman-contributor-free` (*Iceman Contributor Free* on OpenCode Go)
- **`options`:** `{}` (default effort; no custom effort/variant overrides)

## 4. Delegation Rules

1. Every delegated task must state which crate/layer (`backend/`, `bridge/`, `frontend/`) it owns, which invariants apply, and which gates (A–H in [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md)) must pass.
2. Separate parallel implementation tasks writing to overlapping scopes should use `isolation: "worktree"` or strict directory boundaries (`sweeper-winfs`, `sweeper-core`, `sweeper-knowledge`, `sweeper-contract`, `sweeper-desktop`, `frontend`).
3. Code review (`clean-code-guard`), test review (`test-guard`), and doc review (`docs-guard`) are dispatched as separate `review` sub-agent runs using the same standard sub-agent model configuration.
4. Every sub-agent prompt must specify a repo-root Cargo target directory (for example `$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) "target")`) so temporary worktrees on `C:` never accumulate duplicate `target/` caches, and Playwright tests must use `channel: "msedge"`.
5. After every ticket lands/merges (and on each periodic Watchdog tick), run `powershell -ExecutionPolicy Bypass -File scripts/cleanup-artifacts.ps1` to clean unneeded transient build/test/mutation outputs strictly inside the repository root.
