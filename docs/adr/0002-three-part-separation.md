---
status: accepted
---

# Three separated parts: frontend, Rust backend, and a typed bridge

Sweeper is split into `frontend/` (React and TypeScript), `backend/` (Rust crates with all sensitive logic), and `bridge/` (a Tauri shell plus a contract crate that generates the TypeScript types). The frontend never sends file paths for removal, only IDs of scan results held by the backend, so a compromised or buggy UI cannot name a path to delete.

We considered a pure-Rust UI (egui), which is lighter and has a smaller attack surface, and chose Tauri because the user wants design freedom and room to grow the product later. The accepted costs are higher memory use and a WebView dependency; the mitigations are strict capabilities, a strict CSP, and no filesystem or shell plugins.

## Consequences

- `sweeper-contract` is the single source of truth for command shapes; the generated TypeScript is committed and CI fails if it drifts.
- Dependency direction is fixed: `winfs` ← `core` → `knowledge`; the bridge depends on `core` and `contract`; the frontend depends only on the generated contract.
