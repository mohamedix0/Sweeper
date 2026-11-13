# Sweeper

A safety-first Windows tool that finds reclaimable disk space, shows the user exactly what it would remove and why, and removes only what the user confirms. The name is deliberately not tied to one drive, so the product can grow beyond C:.

## Language

### Targets

**Target**:
A known location (a folder, or specific subfolders) that the tool recognises as reclaimable, together with its description, Risk, and Tier.
_Avoid_: Rule, entry, junk

**Allowlist**:
The built-in set of all Targets the tool is permitted to clean. Anything not in it is never removed by default.
_Avoid_: Whitelist, rules list

**Protected Path**:
A location that can never be removed, regardless of the Allowlist, custom paths, or the Analyzer. Fixed in the program and not changeable from the interface.
_Avoid_: Blacklist, excluded path, denylist

**Custom Path**:
A location the user adds themselves in advanced settings to be cleaned. Only its contents are removed, never the folder itself.
_Avoid_: User rule, extra path

**Knowledge Base**:
The built-in catalogue describing known locations: what each is, whether it is temporary, and what removing it risks. The Allowlist is the removable part of it.
_Avoid_: Database, definitions file

### Classification

**Risk**:
The classification shown for every item: Safe, Caution, Unknown, or Protected.
_Avoid_: Severity, danger level

**Safe**:
A known Target whose contents are regenerated automatically and cause no loss when removed. Enabled by default.

**Caution**:
A known Target that is recoverable but costly to regenerate, or could disturb tools. Removable only when its Tier is switched on in settings.

**Unknown**:
A location the tool does not recognise. Its size is shown, but it is removable only after the user turns on "Allow deleting unknown folders" and confirms, and it must still pass Custom Path rules.

**Protected**:
An item or location belonging to a Protected Path (or a non-Allowlist descendant of a protected container root). Displayed for context only and never selectable for Clean.

**Tier**:
A group of Targets that can be switched on or off together in settings (1: system temp and caches, 2: developer caches, 3: heavy or risky items).
_Avoid_: Level, category

### Workflow

**Scan**:
The read-only pass that discovers Targets, measures them, and removes nothing.
_Avoid_: Analyze (reserved for the Analyzer), search

**Preview**:
The screen showing what a Clean would remove, grouped by Target with sizes, where the user can deselect items.
_Avoid_: Dry run, summary

**Clean**:
The confirmed, permanent removal of the previewed items. Bypasses the Recycle Bin.
_Avoid_: Delete all, wipe, purge

**Analyzer**:
The standalone read-only screen showing where space is used across the drive, explaining each item and its Risk, from which the user may select removable items.
_Avoid_: Explorer, disk usage view

**Minimum Age**:
The time since last modification a file must exceed before it is eligible for a Clean (default 24 hours, adjustable).
_Avoid_: Retention, cooldown

**Skipped**:
The outcome for an item that was not removed because it is locked, too recent, or failed re-validation. Always recorded, never forced.
_Avoid_: Failed, ignored

**Link Point**:
A symlink or junction. The tool never follows one; if found inside a Target, only the link itself may be removed, never what it points to.
_Avoid_: Shortcut, reparse point
