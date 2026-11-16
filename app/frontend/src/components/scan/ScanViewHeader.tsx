import type { Settings } from '../../api/client';
import { formatBytes } from '../../utils/format';
import { InfoTip } from '../cleanShared';
import { RefreshIcon } from '../Icons';

interface Props {
  hasResults: boolean;
  heroReclaimable: string;
  tier1: boolean;
  tier2: boolean;
  scanning: boolean;
  cleaning: boolean;
  settings: Settings | null;
  settingsStale: boolean;
  hasActiveScope: boolean;
  onTier1: (v: boolean) => void;
  onTier2: (v: boolean) => void;
  onRunScan: () => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function scopeChipClass(args: { active: boolean }): string {
  if (args.active) return 'scan-scope-chip scan-scope-pill scan-scope-config-chip active';
  return 'scan-scope-chip scan-scope-pill scan-scope-config-chip';
}

function scopeLabelText(args: { active: boolean; label: string }): string {
  if (args.active) return `✓ ${args.label}`;
  return args.label;
}

function ScopePill(props: { active: boolean; label: string; aria: string; disabled: boolean; tipLabel: string; title: string; body: string; onChange: (v: boolean) => void }) {
  return (
    <span className="scan-scope-inline-item">
      <span className={scopeChipClass({ active: props.active })}>
        <label className="scan-scope-pill-label">
          <input className="sr-only" type="checkbox" checked={props.active} onChange={(e) => props.onChange(e.target.checked)} disabled={props.disabled} aria-label={props.aria} />
          <span className="scan-scope-pill-text">{scopeLabelText({ active: props.active, label: props.label })}</span>
        </label>
        <span className="scan-scope-pill-info"><InfoTip label={props.tipLabel}><p><strong>{props.title}</strong><br />{props.body}</p></InfoTip></span>
      </span>
    </span>
  );
}

function deepCleanupText(args: { on: boolean }): string {
  if (args.on) return 'Deep Cleanup: On';
  return 'Deep Cleanup: Off';
}

function deepCleanupAria(args: { on: boolean }): string {
  if (args.on) return 'Deep Cleanup: On · Settings';
  return 'Deep Cleanup: Off · Configure';
}

function deepCleanupTitle(args: { on: boolean }): string {
  if (args.on) return 'Deep Cleanup is on (configure in Settings)';
  return 'Configure Deep Cleanup in Settings';
}

function deepCleanupClass(args: { on: boolean }): string {
  if (args.on) return 'scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip active';
  return 'scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip';
}

function DeepCleanupButton(props: { on: boolean; onOpenSettings: NonNullable<Props['onOpenSettings']> }) {
  return (
    <button type="button" className={deepCleanupClass({ on: props.on })} onClick={() => props.onOpenSettings('enableTier3')} aria-label={deepCleanupAria({ on: props.on })} title={deepCleanupTitle({ on: props.on })}>
      <span className="scan-scope-pill-label scan-scope-pill-label-static"><span className="scan-scope-pill-text">{deepCleanupText({ on: props.on })} <span aria-hidden="true">⚙</span></span></span>
    </button>
  );
}

function staticDeepCleanupClass(args: { on: boolean }): string {
  if (args.on) return 'scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip active on';
  return 'scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip off';
}

function DeepCleanupStatic(props: { on: boolean }) {
  return <span className={staticDeepCleanupClass({ on: props.on })} aria-label={deepCleanupText({ on: props.on })}><span className="scan-scope-pill-label scan-scope-pill-label-static"><span className="scan-scope-pill-text">{deepCleanupText({ on: props.on })} <span aria-hidden="true">⚙</span></span></span></span>;
}

function DeepCleanupScope(props: { settings: Settings | null; onOpenSettings?: Props['onOpenSettings'] }) {
  const on = !!props.settings?.enableTier3;
  if (props.onOpenSettings) return <DeepCleanupButton on={on} onOpenSettings={props.onOpenSettings} />;
  return <DeepCleanupStatic on={on} />;
}

function rescanClass(args: { hasResults: boolean }): string {
  if (args.hasResults) return 'btn secondary-action scan-rescan-btn';
  return 'btn primary scan-rescan-btn';
}

function rescanDisabled(args: { busy: boolean; hasActiveScope: boolean }): boolean {
  if (args.busy) return true;
  return !args.hasActiveScope;
}

function RescanButton(props: { hasResults: boolean; busy: boolean; hasActiveScope: boolean; onRunScan: () => void }) {
  const disabled = rescanDisabled({ busy: props.busy, hasActiveScope: props.hasActiveScope });
  if (props.hasResults) {
    return <button key="rescan" className={rescanClass({ hasResults: true })} onClick={props.onRunScan} disabled={disabled} aria-label="Start Scan"><><RefreshIcon aria-hidden="true" /> <span>Rescan</span></></button>;
  }
  return <button key="scan" className={rescanClass({ hasResults: false })} onClick={props.onRunScan} disabled={disabled} aria-label="Start Scan">Start Scan</button>;
}

function HeroSubtitle(props: { show: boolean; bytes: string }) {
  if (!props.show) return null;
  return <span className="header-subtitle num" aria-label="Reclaimable total">{formatBytes({ bytes: props.bytes })} reclaimable</span>;
}

function SettingsStalePill(props: { stale: boolean }) {
  if (!props.stale) return null;
  return <span className="settings-updated-pill" role="status">Settings updated — Rescan to apply</span>;
}

function ScopeControls(props: { tier1: boolean; tier2: boolean; busy: boolean; settings: Settings | null; onTier1: (v: boolean) => void; onTier2: (v: boolean) => void; onOpenSettings?: Props['onOpenSettings'] }) {
  return (
    <div className="scan-header-scopes scan-header-scopes-inline scan-header-scopes-subtle" aria-label="Scan scopes" title="Scope changes apply on Rescan">
      <span className="scan-scope-label muted">Scan Scope:</span>
      <div className="scan-scope-inline-controls scan-scope-inline-controls-subtle" role="group" aria-label="Scan scopes">
        <ScopePill active={props.tier1} label="Temp Files" aria="Temp Files" disabled={props.busy} tipLabel="About System & App Temp (Tier 1)" title="System & App Temp (Tier 1)" body="Windows Temp, crash dumps & app caches older than 24h" onChange={props.onTier1} />
        <ScopePill active={props.tier2} label="Dev Caches" aria="Dev Caches" disabled={props.busy} tipLabel="About Developer Caches (Tier 2)" title="Developer Caches (Tier 2)" body="npm, Cargo, pip & build caches that rebuild automatically. Scope changes apply on the next Rescan. Matches Settings → General." onChange={props.onTier2} />
        <DeepCleanupScope settings={props.settings} onOpenSettings={props.onOpenSettings} />
      </div>
    </div>
  );
}

function HeaderActions(props: { tier1: boolean; tier2: boolean; busy: boolean; settings: Settings | null; settingsStale: boolean; hasResults: boolean; hasActiveScope: boolean; onTier1: (v: boolean) => void; onTier2: (v: boolean) => void; onRunScan: () => void; onOpenSettings?: Props['onOpenSettings'] }) {
  return (
    <div className="page-header-right scan-header-actions scan-header-actions-v19">
      <ScopeControls tier1={props.tier1} tier2={props.tier2} busy={props.busy} settings={props.settings} onTier1={props.onTier1} onTier2={props.onTier2} onOpenSettings={props.onOpenSettings} />
      <div className="scan-header-rescan-wrap">
        <SettingsStalePill stale={props.settingsStale} />
        <RescanButton hasResults={props.hasResults} busy={props.busy} hasActiveScope={props.hasActiveScope} onRunScan={props.onRunScan} />
      </div>
    </div>
  );
}

export function ScanViewHeader(props: Props) {
  const busy = props.scanning || props.cleaning;
  return (
    <header className="page-header scan-header-row" aria-label="Scan and Clean header">
      <div className="page-header-left scan-header-title-block">
        <div className="page-title-row"><h1 className="page-title" id="scan-heading">Scan &amp; Clean</h1><HeroSubtitle show={props.hasResults} bytes={props.heroReclaimable} /></div>
      </div>
      <HeaderActions tier1={props.tier1} tier2={props.tier2} busy={busy} settings={props.settings} settingsStale={props.settingsStale} hasResults={props.hasResults} hasActiveScope={props.hasActiveScope} onTier1={props.onTier1} onTier2={props.onTier2} onRunScan={props.onRunScan} onOpenSettings={props.onOpenSettings} />
    </header>
  );
}
