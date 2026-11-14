---
colors:
  workspace:
    --bg-workspace-start: "#060b14"
    --bg-workspace-end: "#0b1322"
  glass:
    --bg-glass: "rgba(15, 23, 42, 0.64)"
    --bg-glass-strong: "rgba(11, 19, 34, 0.78)"
    --bg-inset: "rgba(255, 255, 255, 0.03)"
    --bg-inset-hover: "rgba(255, 255, 255, 0.055)"
    --bg-topbar: "rgba(8, 14, 26, 0.72)"
  borders:
    --border-hairline: "rgba(148, 163, 184, 0.16)"
    --border-divider: "rgba(148, 163, 184, 0.11)"
    --border-strong: "rgba(148, 163, 184, 0.24)"
  text:
    --text-primary: "#f1f5f9"
    --text-secondary: "#cbd5e1"
    --text-muted: "#94a3b8"
    --text-on-accent: "#f8fafc"
  accent:
    --accent-cobalt: "#0ea5e9"
    --accent-teal: "#14b8a6"
    --accent-primary: "#0284c7"
    --accent-primary-hover: "#0369a1"
  risk_safe:
    --risk-safe-text: "#6ee7b7"
    --risk-safe-bg: "rgba(16, 185, 129, 0.14)"
    --risk-safe-border: "rgba(110, 231, 183, 0.28)"
  risk_caution:
    --risk-caution-text: "#fcd34d"
    --risk-caution-bg: "rgba(245, 158, 11, 0.14)"
    --risk-caution-border: "rgba(252, 211, 77, 0.28)"
  risk_unknown:
    --risk-unknown-text: "#fdba74"
    --risk-unknown-bg: "rgba(249, 115, 22, 0.14)"
    --risk-unknown-border: "rgba(253, 186, 116, 0.28)"
  risk_protected:
    --risk-protected-text: "#cbd5e1"
    --risk-protected-bg: "rgba(148, 163, 184, 0.14)"
    --risk-protected-border: "rgba(203, 213, 225, 0.24)"
  danger_info:
    --color-danger: "#dc2626"
    --color-danger-hover: "#b91c1c"
    --color-danger-text: "#fca5a5"
    --color-danger-bg: "rgba(220, 38, 38, 0.14)"
    --color-danger-border: "rgba(248, 113, 113, 0.28)"
    --color-danger-on: "#fef2f2"
    --color-info-text: "#7dd3fc"
    --color-info-bg: "rgba(14, 165, 233, 0.12)"
    --color-info-border: "rgba(125, 211, 252, 0.24)"
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
    --text-xl: "1.375rem"
rounded:
  --radius-sm: "6px"
  --radius-md: "8px"
  --radius-lg: "14px"
  --radius-pill: "999px"
spacing:
  --space-1: "0.25rem"
  --space-2: "0.5rem"
  --space-3: "0.75rem"
  --space-4: "1rem"
  --space-5: "1.25rem"
  --space-6: "1.5rem"
components:
  - Topbar
  - Navigation
  - Panel
  - Buttons
  - Notices
  - Badges
  - TargetCard
  - AnalyzerTree
  - Drawer
  - Progress
  - Table
  - LogViewer
  - Forms
  - CleanFooter
---

# Sweeper Design System

## Overview

This document describes the implemented **Obsidian Frosted Glass** design system for the Sweeper desktop UI as defined in source. The UI is a dark, glassmorphic desktop utility surface optimized for scanability, restraint, and explicit safety state.

**Source of truth:** `app/frontend/src/styles/tokens.css`, `app/frontend/src/App.css`, `app/frontend/src/components/Icons.tsx`, `app/frontend/src/components/AnalyzerView.tsx`, `app/frontend/src/components/LogsView.tsx`, `app/frontend/src/components/SettingsView.tsx`, `app/frontend/src/components/cleanShared.tsx`, `app/frontend/index.html`.

**Brand and application icons:**
- Desktop shell assets: `app/bridge/sweeper-desktop/icons/32x32.png`, `app/bridge/sweeper-desktop/icons/128x128.png`, `app/bridge/sweeper-desktop/icons/128x128@2x.png`, `app/bridge/sweeper-desktop/icons/icon.png`, `app/bridge/sweeper-desktop/icons/icon.ico`
- Frontend assets: `app/frontend/src/assets/sweeper-icon.png`, `app/frontend/public/sweeper-icon.png`, `app/frontend/public/favicon.ico`
- HTML icon links (`app/frontend/index.html`): `<link rel="icon" type="image/png" href="/sweeper-icon.png" />` and `<link rel="alternate icon" href="/favicon.ico" />`

**CSP constraints:** `style-src 'self'; img-src 'self' data:; font-src 'self'` (no external font CDNs, no inline `<style>` tags).

## Colors

All color tokens are defined in `:root` in `app/frontend/src/styles/tokens.css` with `color-scheme: dark`.

### Workspace & Glass Surfaces
| Token | Value | Role |
|---|---|---|
| `--bg-workspace-start` | `#060b14` | Workspace gradient start |
| `--bg-workspace-end` | `#0b1322` | Workspace gradient end |
| `--bg-glass` | `rgba(15, 23, 42, 0.64)` | Primary frosted panel background |
| `--bg-glass-strong` | `rgba(11, 19, 34, 0.78)` | Strong glass/inset surface token |
| `--bg-inset` | `rgba(255, 255, 255, 0.03)` | Inset rows, inputs, subtle surfaces |
| `--bg-inset-hover` | `rgba(255, 255, 255, 0.055)` | Hover state for inset rows |
| `--bg-topbar` | `rgba(8, 14, 26, 0.72)` | Sticky topbar glass surface |

Body background (`app/frontend/src/App.css`): `radial-gradient(900px 500px at 10% -10%, rgba(14, 165, 233, 0.16), transparent 60%), radial-gradient(700px 420px at 90% 0%, rgba(20, 184, 166, 0.14), transparent 55%), linear-gradient(180deg, var(--bg-workspace-start), var(--bg-workspace-end))` with `background-attachment: fixed`.

### Borders & Dividers
| Token | Value |
|---|---|
| `--border-hairline` | `rgba(148, 163, 184, 0.16)` |
| `--border-divider` | `rgba(148, 163, 184, 0.11)` |
| `--border-strong` | `rgba(148, 163, 184, 0.24)` |

### Text
| Token | Value |
|---|---|
| `--text-primary` | `#f1f5f9` |
| `--text-secondary` | `#cbd5e1` |
| `--text-muted` | `#94a3b8` |
| `--text-on-accent` | `#f8fafc` |

### Accents & Interactive
| Token | Value | Usage |
|---|---|---|
| `--accent-cobalt` | `#0ea5e9` | Focus/selection accents, progress gradient start |
| `--accent-teal` | `#14b8a6` | Progress gradient end, decorative accents |
| `--accent-primary` | `#0284c7` | Primary button gradient start |
| `--accent-primary-hover` | `#0369a1` | Primary button hover gradient start |

### Semantic `Risk`, Danger, and Info
| Token | Value | Usage |
|---|---|---|
| `--risk-safe-text` | `#6ee7b7` | `Safe` / positive badge text |
| `--risk-safe-bg` | `rgba(16, 185, 129, 0.14)` | `Safe` badge/notice surface |
| `--risk-safe-border` | `rgba(110, 231, 183, 0.28)` | `Safe` border |
| `--risk-caution-text` | `#fcd34d` | `Caution` text |
| `--risk-caution-bg` | `rgba(245, 158, 11, 0.14)` | `Caution` surface |
| `--risk-caution-border` | `rgba(252, 211, 77, 0.28)` | `Caution` border |
| `--risk-unknown-text` | `#fdba74` | `Unknown` text |
| `--risk-unknown-bg` | `rgba(249, 115, 22, 0.14)` | `Unknown` surface |
| `--risk-unknown-border` | `rgba(253, 186, 116, 0.28)` | `Unknown` border |
| `--risk-protected-text` | `#cbd5e1` | `Protected` text |
| `--risk-protected-bg` | `rgba(148, 163, 184, 0.14)` | `Protected` surface |
| `--risk-protected-border` | `rgba(203, 213, 225, 0.24)` | `Protected` border |
| `--color-danger` | `#dc2626` | Destructive CTA base fill |
| `--color-danger-hover` | `#b91c1c` | Destructive CTA hover/base end |
| `--color-danger-text` | `#fca5a5` | Error/danger text |
| `--color-danger-bg` | `rgba(220, 38, 38, 0.14)` | Error/danger surface |
| `--color-danger-border` | `rgba(248, 113, 113, 0.28)` | Error/danger border |
| `--color-danger-on` | `#fef2f2` | Text on destructive CTA |
| `--color-info-text` | `#7dd3fc` | Info text |
| `--color-info-bg` | `rgba(14, 165, 233, 0.12)` | Info surface |
| `--color-info-border` | `rgba(125, 211, 252, 0.24)` | Info border |

Semantic badge classes (`.risk-Safe`, `.risk-Caution`, `.risk-Unknown`, `.risk-Protected`) and destructive CTA styles (`.btn.danger`, `.btn.danger.primary`) are implemented in `app/frontend/src/App.css` and applied via `riskClass` in `app/frontend/src/components/cleanShared.tsx`.

## Typography

**Font stacks (`tokens.css`):**
- `--font-sans: 'Segoe UI Variable Text', 'Segoe UI', system-ui, -apple-system, sans-serif`
- `--font-mono: 'Cascadia Mono', 'Cascadia Code', ui-monospace, SFMono-Regular, Consolas, monospace`

Global body uses `var(--font-sans)`, `font-size: var(--text-base)` (`0.875rem`), `line-height: 1.5`.

**Type scale:**
| Token | Value |
|---|---|
| `--text-xs` | `0.75rem` |
| `--text-sm` | `0.8125rem` |
| `--text-base` | `0.875rem` |
| `--text-md` | `1rem` |
| `--text-lg` | `1.125rem` |
| `--text-xl` | `1.375rem` |

**Usage highlights (from `App.css`):** `.brand` uses `font-size: var(--text-md)` and `font-weight: 650`; panel headings `.panel h2` `var(--text-lg)`/`650`, `.panel h3` `var(--text-md)`/`600`; nav/buttons/body controls `var(--text-base)`; tables, log, item paths/meta `var(--text-sm)`; badges/meta chips `var(--text-xs)` (with analyzer badges overridden to `font-size: 0.72rem`).

**Numeric alignment:** `.num, .item-size, .target-stats, .table td, .kv dd` use `font-variant-numeric: tabular-nums`.

## Layout

- **App shell:** `.app-shell` is `min-height: 100vh`, flex column.
- **Topbar:** sticky (`top: 0`, `z-index: 30`), flex row with space-between, `gap: var(--space-4)`, `padding: var(--space-3) var(--space-6)`.
- **Main container:** `.main` has `padding: var(--space-6)`, `max-width: 1200px`, `width: 100%`, centered via `margin: 0 auto`.
- **Common grids:** `.grid-2` defaults to `grid-template-columns: minmax(320px, 0.85fr) minmax(0, 1.35fr)` with `gap: var(--space-4)`; at `@media (min-width: 980px)` becomes `minmax(420px, 1.05fr) minmax(0, 1.25fr)`; at `@media (max-width: 900px)` collapses to one column.
- **Analyzer tree geometry (`App.css` / `AnalyzerView.tsx`):**
  - Viewport: `.analyzer-viewport` `height: 432px`, `overflow: auto`, `border: 1px solid var(--border-divider)`, `border-radius: var(--radius-md)`, `background: rgba(255, 255, 255, 0.02)`.
  - Row: `.analyzer-row` `display: grid`, `grid-template-columns: minmax(0, 1fr) 72px 44px 78px minmax(236px, max-content)`, `gap: 10px`, `align-items: center`, `height: 54px`, `padding-right: var(--space-2)`, `border-bottom: 1px solid var(--border-divider)`, `box-sizing: border-box`, `overflow: hidden`.
  - Virtualization constants in `AnalyzerView.tsx`: `rowHeight = 54`, `viewportHeight = 432`, `overscan = 8`.
  - Responsive fallback `@media (max-width: 1100px)`: analyzer rows become single-column with `height: auto`, `min-height: 54px`, `padding-top: 8px`, `padding-bottom: 8px`.
- **Item rows (clean/shared lists):** `.item-row` grid `auto minmax(0, 1fr) 120px auto auto` with `gap: 12px`, `padding: 12px 10px`; collapses to `auto 1fr` under `@media (max-width: 900px)`.

## Elevation & Depth

Obsidian Frosted Glass depth is implemented with layered translucent surfaces, backdrop blur, gradients, and shadows:

- **Shadows/tokens:** `--shadow-glass: 0 16px 36px -12px rgba(2, 6, 23, 0.6)`, `--shadow-soft: 0 8px 20px -10px rgba(2, 6, 23, 0.55)`, `--inset-specular: inset 0 1px 0 rgba(255, 255, 255, 0.08)`.
- **Focus ring:** `--focus-ring: 0 0 0 2px rgba(14, 165, 233, 0.65), 0 0 0 4px rgba(14, 165, 233, 0.18)`; applied to interactive focus-visible states and controls.
- **Blur values:** `.topbar` uses `backdrop-filter: blur(22px) saturate(140%)`; `.panel` uses `backdrop-filter: blur(18px) saturate(145%)`; `.badge` uses `blur(8px)`; `.drawer-backdrop` uses `blur(4px)`; `.drawer` uses `blur(28px) saturate(160%)` (all with matching `-webkit-backdrop-filter`).
- **Surface layering:** panels use `var(--bg-glass)` + `var(--shadow-glass)` + `var(--inset-specular)`; nested panels become transparent with divider separation; drawer is a stronger glass sheet (`rgba(10, 18, 34, 0.84)`) with left cyan border and deep shadow.

## Shapes

- **Radii:** `--radius-sm: 6px`, `--radius-md: 8px`, `--radius-lg: 14px`, `--radius-pill: 999px`.
- **Applications:** nav pill container/buttons and badges/meta chips use pill radii; panels use `--radius-lg`; buttons/inputs/notices/log/table wrappers use `--radius-md`; icons/badges/tree toggles use small radii (`8px`, `10px`, `--radius-sm` as defined in CSS).
- **Brand icon:** `.brand-icon` is `28px × 28px` with `border-radius: 8px` and shadow in `App.css`.

## Components

- **Topbar / Navigation (`.topbar`, `.brand`, `.nav`)**: sticky frosted bar with brand icon/text and segmented pill navigation; active nav item uses `rgba(14, 165, 233, 0.14)` background, `rgba(125, 211, 252, 0.22)` border, and specular inset.
- **Panels (`.panel`)**: frosted glass containers with 14px radius and depth shadows; nested panels flatten into section dividers.
- **Buttons (`.btn`, `.btn.primary`, `.btn.danger`)**: neutral glass buttons; primary uses cobalt-to-teal gradient (`linear-gradient(135deg, var(--accent-primary), var(--accent-teal))`); destructive `Clean Permanently` CTA uses red gradient (`linear-gradient(180deg, var(--color-danger), var(--color-danger-hover))` / hover `linear-gradient(180deg, #ef4444, var(--color-danger))`) to remain visually distinct from read-only scan/preview actions.
- **Notices & Badges (`.notice`, `.badge`, `.risk-*`)**: semantic tinted surfaces for `warn/error/info` and `Risk` states (`Safe`, `Caution`, `Unknown`, `Protected`) driven by tokenized text/bg/border colors.
- **Analyzer (`AnalyzerView.tsx`)**: expandable, virtualized tree viewport (`432px`, `54px` rows, 5-column grid) with selectable nodes, explain action, risk/tier/link/display-only badges, and integrated Preview/Clean flow including typed confirmation gate (`DELETE`).
- **Explain Drawer (`cleanShared.tsx`)**: right-side modal drawer (`role="dialog"`, `aria-modal="true"`) with backdrop blur, metadata card, and risk badge presentation.
- **Progress (`.progress`, `.progress-bar`)**: 8px rounded track with cobalt-to-teal fill and indeterminate sweep animation.
- **Tables & Logs (`.table`, `.logs-table`, `.log-table-wrap`, `pre.log`)**: low-contrast glass tables with sticky-like header styling, active log row highlight, and monospace dark log viewer (`LogsView.tsx`).
- **Forms & Settings (`SettingsView.tsx`, inputs/checkboxes)**: glass inputs (`rgba(255, 255, 255, 0.03)`), focus accent, disabled states, settings sections, custom path table, and elevation/tier controls with semantic notices.
- **Icons (`Icons.tsx`)**: inline SVG icons (`ChevronRightIcon`, `ChevronDownIcon`, `DotIcon`) using a shared 16×16 stroked base (`strokeWidth: 1.75`, round caps/joins), `currentColor`, and accessibility defaults (`aria-hidden`, `focusable="false"`).

## Do's and Don'ts

- **Do** use the tokenized Obsidian Frosted Glass palette and surfaces from `tokens.css` (`--bg-glass`, `--bg-topbar`, `--border-*`, semantic `--risk-*` and `--color-*`) instead of introducing ad hoc colors.
- **Do** preserve frosted depth cues: backdrop blur, `--shadow-glass`/`--shadow-soft`, and `--inset-specular` on primary glass surfaces.
- **Do** keep the Analyzer geometry consistent with implementation (`432px` viewport, `54px` row height, grid `minmax(0, 1fr) 72px 44px 78px minmax(236px, max-content)`).
- **Do** keep semantic safety signaling explicit: `Risk` badges use `riskClass`/`.risk-*`, and irreversible deletion uses the destructive `.btn.danger` CTA distinct from scan/preview actions.
- **Do** maintain CSP-compatible styling and assets: `style-src 'self'; img-src 'self' data:; font-src 'self'`, local fonts/assets only.
- **Don't** add external font CDNs, inline `<style>` tags, or non-local icon/image dependencies that violate CSP constraints.
- **Don't** document or introduce personal/machine-specific paths, usernames, emails, or IDs in docs or examples; keep references repo-relative and neutral.
- **Don't** blur semantic meaning of `Risk` states or present deletion actions with non-destructive styling.
