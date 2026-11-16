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
  - Driving `$to-spec` → `$to-tickets` → `$implement-spec` → `$code-review` → PR creation (`gh pr create --base main`) → CI, CodeScene & CodeRabbit review continuously across Phases 0–6, pausing for merge decision after each PR is reviewed (and pausing for any human-only irreversible action such as running live deletion outside the sentinel test sandbox on real `C:` data). **Any new medium-to-large feature or major bug must be tracked on GitHub with `$to-spec` and `$to-tickets` before implementation.**

## 3. Standard Sub-Agent Model Configuration

All sub-agents (implementation, research, testing, design, and review) must use the following target configuration unless the user explicitly specifies another model or effort for a particular task:

- **`providerInstanceId`:** `opencode`
- **`model`:** `opencode-go/hy4-flash-free` (*Hy4 flash Free* on OpenCode Go)
- **`options`:** `{}` (default effort; no custom effort/variant overrides)

## 4. Delegation Rules

1. Every delegated task must state which crate/layer (`backend/`, `bridge/`, `frontend/`) it owns, which invariants apply, and which gates (A–H in [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md)) must pass.
2. Separate parallel implementation tasks writing to overlapping scopes should use `isolation: "worktree"` or strict directory boundaries (`sweeper-winfs`, `sweeper-core`, `sweeper-knowledge`, `sweeper-contract`, `sweeper-desktop`, `frontend`).
3. Code review (`clean-code-guard`), test review (`test-guard`), and doc review (`docs-guard`) are dispatched as separate `review` sub-agent runs using the same standard sub-agent model configuration. When CodeScene tools are available in-session, also enforce `.agents/skills/codescense-review/SKILL.md` (`code_health_review`, `pre_commit_code_health_safeguard`, `analyze_change_set`).
4. **Pull Request, CodeScene & CodeRabbit Gate:** Never push directly to `main`, and **never run the CodeRabbit CLI (`coderabbit`) locally under any circumstances** (use GitHub PR checks/reviews only). Group cohesive tasks or each phase branch into a Pull Request targeting `main` (`gh pr create --base main`), link and watch the PR, and ensure **GitHub Actions CI**, **CodeScene** Code Health review, and **CodeRabbit** PR review (`@coderabbitai review` if skipped on OSS) have all run on the PR before any merge. **Fix EVERY issue reported on the GitHub PR by CI, CodeRabbit, or CodeScene on the PR branch, AND reply on GitHub to every CodeRabbit and CodeScene review comment/thread confirming the fix.** Do not merge into `main` until all three pass/approve with zero unresolved or unanswered comments.
5. Every sub-agent prompt must specify a repo-root Cargo target directory (for example `$env:CARGO_TARGET_DIR = (Join-Path (git rev-parse --show-toplevel) "target")`) so temporary worktrees on `C:` never accumulate duplicate `target/` caches, and Playwright tests must use `channel: "msedge"`.
6. After every ticket/PR lands (and on each periodic Watchdog tick), run `powershell -ExecutionPolicy Bypass -File scripts/cleanup-artifacts.ps1` to clean unneeded transient build/test/mutation outputs strictly inside the repository root.
