---
status: accepted
---

# Deny-by-default removal: Allowlist plus hard Protected Paths

The tool removes only what is in the built-in Allowlist (compiled into the executable) or what the user explicitly adds as a Custom Path. A hard-coded Protected Path list overrides everything, including Custom Paths and the Analyzer, and cannot be changed from the interface. Every path is re-validated at the moment of removal, not trusted from the earlier Scan. A path that does not exist exactly as given removes nothing; there is no fallback to a parent folder.

We chose this over a "remove anything not forbidden" model because accidental loss of unauthorised files is the one failure the project cannot tolerate. The cost is that new cleanable locations need a code change (or a Custom Path), which is accepted.

## Consequences

- Allowlist entries must name the exact subfolder (for example a browser's cache folder, never the browser profile root).
- Link Points are never followed, during Scan, Analyzer, or Clean.
