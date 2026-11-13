---
status: accepted
---

# Removal is handle-based, in one module, with our own walker

Files are removed by opening each entry without following reparse points, enumerating relative to the parent handle, re-checking the opened handle's final path and attributes, and deleting by handle with POSIX semantics. Only `remover` in `sweeper-core` may trigger removal, enforced by a clippy `disallowed_methods` rule and a CI grep gate.

We chose this over `std::fs::remove_dir_all` or path-based Win32 calls because per-item policy (Minimum Age, Skipped, Link Points, cloud placeholders) needs our own walker, and path-based removal re-resolves the path on every call, which allows a junction to be swapped in mid-operation (the class of bug behind CVE-2022-21658). The cost is a small, audited `unsafe` surface confined to `sweeper-winfs`.

## Consequences

- Cloud placeholders (for example OneDrive) are always Skipped, because deleting them propagates to the cloud.
- The Windows API calls need real-NTFS integration tests, including a swap-during-clean race test.
