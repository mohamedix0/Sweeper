# Sweeper Design System

> Status: Active UI & Brand Specification  
> Mode: Desktop Utility (`Operate` — scanability, restraint, explicit safety state)  
> CSP Constraints: `style-src 'self'; img-src 'self' data:; font-src 'self'` (zero external font CDNs, zero inline `<style>` tags)

---

## 1. Brand & Application Icon

- **Visual Mark:** Obsidian rounded-squircle tile (`#0B1220` – `#182235`) framing a faceted cobalt-to-teal protective shield (`#0EA5E9` → `#14B8A6`) bisected by an ice-white diagonal sweep arc.
- **Desktop Shell Assets (`app/bridge/sweeper-desktop/icons/`):**
  - `32x32.png` (window title bar / small taskbar)
  - `128x128.png`, `128x128@2x.png` (256×256)
  - `icon.png` (512×512 master PNG with transparent squircle corners)
  - `icon.ico` (multi-resolution Windows ICO: `256, 128, 64, 48, 32, 24, 16`)
- **Frontend Assets:**
  - `app/frontend/src/assets/sweeper-icon.png` (64×64 crisp PNG imported into `.topbar > .brand`)
  - `app/frontend/public/sweeper-icon.png` + `app/frontend/public/favicon.ico` (referenced via `<link rel="icon">` in `app/frontend/index.html`)

---

## 2. Color System

All colors are defined as CSS custom properties in `app/frontend/src/styles/tokens.css`. Every text/background pairing meets WCAG AA (`>= 4.5:1` for text, `>= 3:1` for focus rings and UI borders).

### 2.1 Brand & Surface Tokens

| Token | Hex | Role |
|---|---|---|
| `--bg-topbar` | `#0B1220` | Obsidian command bar & log viewer surface |
| `--bg-topbar-elevated` | `#162238` | Inactive topbar nav button surface |
| `--bg-canvas` | `#F4F7FA` | Main application workspace background |
| `--bg-surface` | `#FFFFFF` | Primary panels, drawers, tables, target groups |
| `--bg-surface-subtle` | `#EEF2F7` | Table headers, neutral chips, subtle hover states |
| `--bg-surface-inset` | `#F8FAFC` | Inset strips (KV rows, metadata callouts) |
| `--border-subtle` | `#DCE4EE` | Hairline dividers and panel borders |
| `--border-strong` | `#CBD5E1` | Interactive control borders (inputs, secondary buttons) |

### 2.2 Text Hierarchy

| Token | Hex | Contrast on `#FFFFFF` | Role |
|---|---|---|---|
| `--text-primary` | `#0F172A` | `17.4:1` | Headings, target names, primary values |
| `--text-secondary` | `#334155` | `10.3:1` | Descriptions, body text, table cells |
| `--text-muted` | `#475569` | `7.3:1` | Helper copy, timestamps, paths metadata |
| `--text-on-dark` | `#F8FAFC` | `17.9:1` (on `#0B1220`) | Topbar brand, active nav label, log text |
| `--text-on-dark-muted` | `#94A3B8` | `7.1:1` (on `#0B1220`) | Secondary text on obsidian surfaces |

### 2.3 Interactive & Brand Accents

| Token | Hex | Role |
|---|---|---|
| `--accent-primary` | `#0369A1` | Primary CTA fill (`Start Scan`, `Preview Clean`, `Save Settings`) |
| `--accent-primary-hover` | `#075985` | Primary CTA hover |
| `--accent-sky` | `#0EA5E9` | Focus ring (`2px` white gap + `2px` sky halo), dark-surface active border |
| `--accent-teal` | `#0D9488` | Positive completion indicators & active progress accents |
| `--accent-tint` | `#E0F2FE` | Active indicator fill / subtle info surface |

### 2.4 Semantic Safety & `Risk` Palette

| State / `Risk` | Text (`--*-text`) | Surface (`--*-bg`) | Border (`--*-border`) | Usage |
|---|---|---|---|---|
| `Safe` | `#065F46` | `#ECFDF5` | `#A7F3D0` | Confirmed reclaimable cache/temp targets |
| `Caution` | `#92400E` | `#FFFBEB` | `#FDE68A` | Review-recommended targets & age warnings |
| `Unknown` | `#9A3412` | `#FFF7ED` | `#FDBA74` | Advanced-gated unknown items requiring `DELETE` confirmation |
| `Protected` / `Skipped` | `#334155` | `#F1F5F9` | `#CBD5E1` | Non-deletable `Protected Path`s, `Link Point`s, placeholders |
| `Danger` | `#991B1B` | `#FEF2F2` | `#FECACA` | Errors, destructive `Clean Permanently` action (`#B91C1C` solid fill) |
| `Info` | `#075985` | `#F0F9FF` | `#BAE6FD` | Non-blocking operational status notices |

---

## 3. Typography & Type Scale

Native Windows font stack for zero network latency and strict CSP compliance:

- `--font-sans`: `'Segoe UI Variable Text', 'Segoe UI', system-ui, -apple-system, sans-serif`
- `--font-mono`: `'Cascadia Mono', 'Cascadia Code', ui-monospace, SFMono-Regular, Consolas, monospace`

| Token | Size (`rem` / `px`) | Weight / Line Height | Usage |
|---|---|---|---|
| `--text-xs` | `0.75rem` (12px) | `500` / `1.4` | Badges, status chips, path metadata |
| `--text-sm` | `0.8125rem` (13px) | `400`–`500` / `1.45` | Paths (`.mono`), table cells, log lines, helper labels |
| `--text-base` | `0.875rem` (14px) | `400`–`600` / `1.5` | Controls, buttons, inputs, body copy |
| `--text-md` | `1rem` (16px) | `600` / `1.35` | Brand wordmark, target/section titles |
| `--text-lg` | `1.125rem` (18px) | `650` / `1.3` | View headings (`h1`, `h2`) |
| `--text-xl` | `1.375rem` (22px) | `700` / `1.2` | KPI totals (`Bytes freed`, reclaimable summary) |

**Numeric Alignment:** All byte sizes, item counts, timestamps, and percentages use `font-variant-numeric: tabular-nums`.

---

## 4. Spacing, Layout & Radii (4px Base Scale)

| Token | Value | Usage |
|---|---|---|
| `--space-1` | `0.25rem` (4px) | Inline icon/badge gaps |
| `--space-2` | `0.5rem` (8px) | Compact control gaps, table cell padding |
| `--space-3` | `0.75rem` (12px) | Row spacing, field spacing, input padding |
| `--space-4` | `1rem` (16px) | Panel padding, section margins |
| `--space-5` | `1.25rem` (20px) | Major container gaps |
| `--space-6` | `1.5rem` (24px) | Page outer padding |

- **Radii:** `--radius-sm: 4px` (badges, inline code), `--radius-md: 6px` (buttons, inputs, item rows), `--radius-lg: 10px` (panels, drawers, target cards), `--radius-pill: 999px` (elevation/status pills).
- **Container Max Width:** `1200px` centered in `.main`.
- **Focus State:** Every focusable button, nav tab, input, and checkbox uses `:focus-visible` with `outline: none; box-shadow: 0 0 0 2px var(--bg-surface), 0 0 0 4px var(--accent-sky);` (with `--bg-topbar` offset inside the dark topbar).
- **Destructive Action Distinction:** `Clean Permanently` uses `.btn.danger` (`#B91C1C` fill, `#991B1B` hover) so irreversible permanent deletion is visually distinct from read-only `Start Scan` / `Preview Clean` (`--accent-primary`).
