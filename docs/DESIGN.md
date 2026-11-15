---
colors:
  dark:
    --bg-workspace: "#111214"
    --bg-sidebar: "#141619"
    --bg-surface: "#18191D"
    --bg-surface-alt: "#1E2026"
    --bg-inspector: "#18191D"
    --bg-inset: "rgba(255, 255, 255, 0.035)"
    --bg-inset-hover: "rgba(255, 255, 255, 0.06)"
    --bg-header: "rgba(17, 18, 20, 0.92)"
    --bg-overlay: "rgba(0, 0, 0, 0.55)"
    --bg-log: "#121317"
    --border-hairline: "rgba(255, 255, 255, 0.09)"
    --border-subtle: "rgba(255, 255, 255, 0.075)"
    --border-divider: "rgba(255, 255, 255, 0.07)"
    --border-strong: "rgba(255, 255, 255, 0.15)"
    --text-primary: "#F4F5F7"
    --text-secondary: "#B4B9C4"
    --text-muted: "#828998"
    --text-on-accent: "#FFFFFF"
    --accent-primary: "#2563EB"
    --accent-primary-hover: "#1D4ED8"
  light:
    --bg-workspace: "#F4F6FA"
    --bg-sidebar: "#E8EEF4"
    --bg-surface: "#FFFFFF"
    --bg-surface-alt: "#F8FAFC"
    --bg-inspector: "#FFFFFF"
    --bg-inset: "rgba(15, 23, 42, 0.035)"
    --bg-inset-hover: "rgba(15, 23, 42, 0.06)"
    --bg-header: "rgba(255, 255, 255, 0.92)"
    --bg-overlay: "rgba(15, 23, 42, 0.32)"
    --bg-log: "#F8FAFC"
    --border-hairline: "rgba(15, 23, 42, 0.09)"
    --border-subtle: "rgba(15, 23, 42, 0.08)"
    --border-divider: "rgba(15, 23, 42, 0.065)"
    --border-strong: "rgba(15, 23, 42, 0.15)"
    --text-primary: "#0F172A"
    --text-secondary: "#475569"
    --text-muted: "#64748B"
    --text-on-accent: "#FFFFFF"
    --accent-primary: "#2563EB"
    --accent-primary-hover: "#1D4ED8"
  semantic: # illustrative dark/light summary (not literal token names); actual CSS custom properties are defined in app/frontend/src/styles/tokens.css
    risk-safe: # --risk-safe-text/bg/border
      text: "#6EE7B7 (dark) / #047857 (light)"
      bg: "rgba(16,185,129,0.13) (dark) / rgba(16,185,129,0.12) (light)"
      border: "rgba(110,231,183,0.26) (dark) / rgba(4,120,87,0.22) (light)"
    risk-caution: # --risk-caution-text/bg/border
      text: "#FCD34D (dark) / #B45309 (light)"
      bg: "rgba(245,158,11,0.13)"
      border: "rgba(252,211,77,0.26) (dark) / rgba(180,83,9,0.22) (light)"
    risk-unknown: # --risk-unknown-text/bg/border
      text: "#C4B5FD (dark) / #6D28D9 (light)"
      bg: "rgba(139,92,246,0.14) (dark) / rgba(139,92,246,0.11) (light)"
      border: "rgba(196,181,253,0.26) (dark) / rgba(109,40,217,0.22) (light)"
    risk-protected: # --risk-protected-text/bg/border
      text: "#94A3B8 (dark) / #475569 (light)"
      bg: "rgba(148,163,184,0.12) (dark) / rgba(100,116,139,0.11) (light)"
      border: "rgba(148,163,184,0.22) (dark) / rgba(71,85,105,0.20) (light)"
    danger: # --color-danger, --color-danger-hover, --color-danger-text/bg/border/on
      base: "#DC2626"
      hover: "#B91C1C"
      text: "#FCA5A5 (dark) / #B91C1C (light)"
      on: "#FFF5F5"
    info: # --color-info-text/bg/border
      text: "#93C5FD (dark) / #1D4ED8 (light)"
      bg: "rgba(59,130,246,0.12) (dark) / rgba(37,99,235,0.09) (light)"
      border: "rgba(147,197,253,0.24) (dark) / rgba(37,99,235,0.20) (light)"
typography:
  fonts:
    sans: "'Segoe UI Variable Text', 'Segoe UI', system-ui, -apple-system, sans-serif"
    mono: "'Cascadia Mono', 'Cascadia Code', ui-monospace, SFMono-Regular, Consolas, monospace"
  scale:
    --text-xs: "0.75rem"
    --text-sm: "0.8125rem"
    --text-base: "0.875rem"
    --text-md: "1rem"
    --text-lg: "1.125rem"
    --text-xl: "1.25rem"
rounded:
  --radius-sm: "6px"
  --radius-md: "8px"
  --radius-lg: "12px"
  --radius-pill: "999px"
spacing:
  --space-1: "0.25rem"
  --space-2: "0.5rem"
  --space-3: "0.75rem"
  --space-4: "1rem"
  --space-5: "1.25rem"
  --space-6: "1.5rem"
components:
  - AppShell
  - SidebarNavigation
  - PageHeader
  - ScopeBar
  - Buttons
  - Notices
  - Badges
  - ScanList
  - AnalyzerTree
  - InspectorPane
  - ActionDock
  - Progress
  - Tables
  - ActivityLogs
  - SettingsForms
---

# Sweeper Design System

## Overview

This document describes the implemented frontend design system for Sweeper as defined in source. The UI is a native-feeling, desktop-oriented workspace optimized for scanability, restraint, and explicit safety signaling (`Risk`, `Protected`, typed confirmations, irreversible `Clean Permanently`).

**Source of truth:** `app/frontend/src/styles/tokens.css`, `app/frontend/src/App.css`, `app/frontend/src/App.tsx`, view components in `app/frontend/src/components/`, and `app/frontend/index.html`.

**Brand and application icons:**
- Desktop shell assets: `app/bridge/sweeper-desktop/icons/32x32.png`, `app/bridge/sweeper-desktop/icons/128x128.png`, `app/bridge/sweeper-desktop/icons/128x128@2x.png`, `app/bridge/sweeper-desktop/icons/icon.png`, `app/bridge/sweeper-desktop/icons/icon.ico`
- Frontend assets: `app/frontend/src/assets/sweeper-icon.png`, `app/frontend/public/sweeper-icon.png`, `app/frontend/public/favicon.ico`
- HTML icon links (`app/frontend/index.html`): `<link rel="icon" type="image/png" href="/sweeper-icon.png" />` and `<link rel="alternate icon" href="/favicon.ico" />`

**CSP constraints:** the canonical app CSP (source: `app/bridge/sweeper-desktop/tauri.conf.json`) is `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`. Fonts are served locally and fall under `default-src 'self'` (no separate `font-src` directive). Do not add external font/asset CDNs or inline `<style>` tags.

## Themes and Tokens

Design tokens are defined in `app/frontend/src/styles/tokens.css`.

- **Dark theme** (`:root, [data-theme="dark"]`, `color-scheme: dark`): workspace/sidebar/surface palette (`--bg-workspace`, `--bg-sidebar`, `--bg-surface`, `--bg-surface-alt`, `--bg-inspector`, `--bg-inset`, `--bg-log`), border tiers (`--border-hairline/subtle/divider/strong`), text colors, accent/nav colors, semantic `risk-*`, `danger`, and `info` tokens, focus ring, popover shadow, scrollbar, and selection tokens.
- **Light theme** (`[data-theme="light"]`, `color-scheme: light`): parallel token set with light surfaces/text/borders and adjusted semantic contrast values.
- **Shared scales** (`:root`): typography stacks and scale (`--font-sans`, `--font-mono`, `--text-xs`…`--text-xl`), spacing (`--space-1`…`--space-6`), and radii (`--radius-sm/md/lg/pill`).

Notes:
- The active token system is flat surface-based (workspace/sidebar/surface/inset). There are no `--bg-glass`, `--bg-topbar`, or `--shadow-glass` tokens in the current implementation.
- Semantic `Risk`, danger, and info states are tokenized and theme-aware; `.risk-Safe/Caution/Unknown/Protected` and destructive/notice classes in `App.css` consume these tokens.

## Typography

- **Sans:** `--font-sans: 'Segoe UI Variable Text', 'Segoe UI', system-ui, -apple-system, sans-serif`
- **Mono:** `--font-mono: 'Cascadia Mono', 'Cascadia Code', ui-monospace, SFMono-Regular, Consolas, monospace`
- **Base body:** `var(--text-base)` (`0.875rem`), line-height `1.5`, antialiased text rendering.
- **Scale:** `--text-xs` 0.75rem, `--text-sm` 0.8125rem, `--text-base` 0.875rem, `--text-md` 1rem, `--text-lg` 1.125rem, `--text-xl` 1.25rem.
- **Numeric alignment:** byte counts and tabular numeric UI use `font-variant-numeric: tabular-nums` (e.g. `.num`, `.item-size`, `.table td`, inspector/meta values).
- **Usage patterns:** page titles ~17–18px/600, section/row headings `--text-sm`–13px semibold, metadata and controls `--text-xs`/`--text-sm`, monospace for paths/log/code-like values where implemented.

## Layout and Workspace

- **Shell:** `.app-shell` is a full-height grid `216px` sidebar + flexible main area (`grid-template-columns: 216px minmax(0, 1fr)`), `100vh` desktop workspace.
- **Sidebar:** left rail (`.sidebar`, `--bg-sidebar`) with brand, grouped primary navigation, and footer privilege/status controls.
- **Navigation groups (`App.tsx`):**
  - **Workspace:** `Scan & Clean`, `Analyzer`
  - **System:** `Settings`, `Activity` (logs view)
- **Main area:** `.app-main` column with `.main-content` (compact padding, scroll-managed content) hosting view stacks/panels. Views are shown/hidden or conditionally rendered per navigation state.
- **Responsive behavior:** layout collapses to a single-column shell on narrow widths (`@media (max-width: 900px)` stacks sidebar above main and adjusts navigation flow). Master/detail and logs layouts also collapse at breakpoints defined in `App.css` (e.g. analyzer/scan/log panes become stacked on smaller widths).
- **Density:** compact toolbars, list rows, and inspector panes favor information density while preserving readable hit targets and focus visibility (`--focus-ring`).

## Surfaces, Shapes, and Interaction

- **Radii:** `--radius-sm` (6px), `--radius-md` (8px), `--radius-lg` (12px), `--radius-pill` (999px) used for inputs/buttons/cards, grouped controls/chips, and badges/toggles as appropriate.
- **Borders/dividers:** hairline/subtle/divider/strong tokens separate panes, rows, toolbars, and form sections without heavy elevation.
- **Depth cues:** primarily border, background layering, and `--shadow-popover` for overlays/popovers/drawers; focus states use `--focus-ring`.
- **Controls:** neutral buttons default to bordered surface styling; primary actions use accent fill (`--accent-primary` / hover), destructive permanent actions use danger styling (`.btn.danger` / `.danger.primary`) and must remain visually distinct from read-only scan/preview actions.
- **States:** disabled controls and rows are dimmed; inspected/selected states use subtle highlight backgrounds/left indicators in scan/analyzer lists. Theme-aware scrollbar and text selection styles are applied globally.

## Key Components and Views

- **Scan & Clean:** scope/filter bar, target/group and item rows with selection, status/risk indicators, progress feedback, and inspector/detail + preview/clean action flow (including typed confirmation when required).
- **Analyzer:** root/location controls and quick presets, allocation/hotspot visualization for analyzed results, filter/sort toolbar, expandable/selectable tree/list rows with share/size/count/status columns, inspector pane, and Preview/Clean integration.
- **Inspector / Explain content:** contextual metadata (size/items/eligibility/status), guidance, path display/copy affordances, and risk/status badges; read-only/protected/display-only states are explicitly surfaced.
- **Settings:** grouped form sections/cards for general/appearance, custom paths (validated add/remove flow), deep cleanup (`enableTier3`) and dependent inactive-project rules, recycle-bin behavior surfaced as included with Deep Cleanup, unknown/custom-path cleanup opt-ins with confirmations, and elevation-gated system-wide cache controls with prerequisite indicators.
- **Activity (Logs):** log list and detail/structured/raw viewing surfaces styled with surface/log tokens and semantic status badges.
- **Shared UI primitives:** notices (`info/warn/error`), badges (including `risk-*`), segmented/chip/toggle controls, checkboxes/switches, inputs/selects, progress bars/meters, tables, and icon buttons.

## Do's and Don'ts

- **Do** use tokens from `tokens.css` and existing component classes/patterns for colors, spacing, radii, and states.
- **Do** keep `Risk`, `Protected`, eligibility, and destructive-action signaling explicit and consistent across Scan, Analyzer, Settings, and confirmations.
- **Do** preserve accessibility semantics: labels, `aria-*` attributes, focus visibility, keyboard-operable controls, and readable contrast in both themes.
- **Do** keep paths/values in monospace or tabular numeric styling where appropriate for scanability.
- **Don't** add external fonts/assets, inline `<style>` tags, or dependencies that violate frontend/CSP constraints.
- **Don't** document or introduce personal/machine-specific data (real usernames, absolute local machine paths, emails, IDs) in docs, examples, or presets.
- **Don't** reintroduce obsolete visual tokens/layout assumptions (e.g. frosted-glass topbar tokens or removed Analyzer geometry) without updating source and this document together.
