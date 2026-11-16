import { formatBytes, pluralize } from '../../utils/format';
import { BrowseIcon, RefreshIcon } from '../Icons';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { AnalyzerStatusFilter } from './analyzerHelpers';

type HeaderProps = {
  hasResult: boolean;
  displayPath: string;
  totalBytes?: string;
  entryCount: number;
  totalItems?: string;
  analyzing: boolean;
  cleaning: boolean;
  canAnalyze: boolean;
  onRun: () => void;
};

function headerSubtitle(props: HeaderProps): string {
  if (props.hasResult) {
    const size = formatBytes({ bytes: props.totalBytes ?? '0' });
    const entries = pluralize({ count: props.entryCount, singular: 'entry', plural: 'entries' });
    return `${props.displayPath} · ${size} across ${entries} (${Number(props.totalItems || '0')} files total) · All files on disk`;
  }
  return `All files & disk hotspots under ${props.displayPath}`;
}

function headerButtonKey(hasResult: boolean): string {
  if (hasResult) return 'reanalyze';
  return 'analyze';
}

function headerButtonClass(hasResult: boolean): string {
  if (hasResult) return 'btn secondary-action';
  return 'btn primary';
}

function HeaderButtonContent(props: { hasResult: boolean }) {
  if (props.hasResult) return <><RefreshIcon aria-hidden="true" /> <span>Re-analyze</span></>;
  return <>Analyze</>;
}

function headerDisabled(props: HeaderProps): boolean {
  if (props.analyzing) return true;
  if (props.cleaning) return true;
  return !props.canAnalyze;
}

export function AnalyzerHeader(props: HeaderProps) {
  const disabled = headerDisabled(props);
  return (
    <header className="page-header" aria-label="Analyzer header">
      <div className="page-header-left analyzer-header-left">
        <div className="page-title-row">
          <h1 className="page-title" id="analyzer-heading">Analyzer</h1>
          <span className="header-subtitle num" aria-label="Analyzer subtitle">{headerSubtitle(props)}</span>
        </div>
      </div>
      <div className="page-header-right">
        <button key={headerButtonKey(props.hasResult)} className={headerButtonClass(props.hasResult)} onClick={props.onRun} disabled={disabled} aria-label="Analyze">
          <HeaderButtonContent hasResult={props.hasResult} />
        </button>
      </div>
    </header>
  );
}

type LocationProps = {
  rootPath: string;
  hasResult: boolean;
  analyzing: boolean;
  cleaning: boolean;
  onRootPath: (v: string) => void;
  onRun: (path?: string) => void;
  onCycleBrowse: () => void;
};

function presetValue(label: 'C:\\' | 'C:\\Users' | 'Temp'): string {
  if (label === 'Temp') return 'C:\\Windows\\Temp';
  return label;
}

function isDefaultRootSelected(args: { label: string; rootPath: string }): boolean {
  if (args.label !== 'C:\\') return false;
  const norm = (p: string) => p.trim().replace(/[\\/]+$/, '').toLowerCase();
  return norm(args.rootPath) === norm('C:\\');
}

function QuickPresets(props: { rootPath: string; analyzing: boolean; cleaning: boolean; hasResult: boolean; onRun: (path: string) => void; onRootPath: (v: string) => void }) {
  const presets = (['C:\\', 'C:\\Users', 'Temp'] as const).filter((label) => !isDefaultRootSelected({ label, rootPath: props.rootPath }));
  return (
    <div className="quick-roots quick-roots-inline" aria-label="Quick root path presets">
      <span className="muted small-note">Quick:</span>
      {presets.map((label) => {
        const value = presetValue(label);
        function onClick() {
          props.onRootPath(value);
          if (props.hasResult) props.onRun(value);
        }
        return <button key={label} type="button" className="chip-btn pill-chip small-chip" onClick={onClick} disabled={props.analyzing || props.cleaning}>{label}</button>;
      })}
    </div>
  );
}

function busyDisabled(input: { analyzing: boolean; cleaning: boolean }): boolean {
  if (input.analyzing) return true;
  return input.cleaning;
}

function goDisabled(props: LocationProps): boolean {
  if (busyDisabled({ analyzing: props.analyzing, cleaning: props.cleaning })) return true;
  return !props.rootPath.trim();
}

export function AnalyzerLocationBar(props: LocationProps) {
  const busy = busyDisabled({ analyzing: props.analyzing, cleaning: props.cleaning });
  return (
    <div className="analyzer-location-bar" aria-label="Analyzer location bar">
      <div className="analyzer-root-group analyzer-root-group-tight">
        <label htmlFor="analyzer-root" className="muted small-note">Root</label>
        <div className="analyzer-root-inline">
          <input id="analyzer-root" type="text" value={props.rootPath} onChange={(e) => props.onRootPath(e.target.value)} aria-label="Analyzer root path" disabled={busy} className="analyzer-root-input" />
          <button type="button" className="btn small analyzer-go-btn" onClick={() => props.onRun()} disabled={goDisabled(props)} aria-label="Go">Go</button>
        </div>
        <button type="button" className="btn browse-btn" onClick={props.onCycleBrowse} disabled={busy} aria-label="Use preset root path"><BrowseIcon /> <span>Use Preset</span></button>
      </div>
      <QuickPresets rootPath={props.rootPath} analyzing={props.analyzing} cleaning={props.cleaning} hasResult={props.hasResult} onRun={(p) => props.onRun(p)} onRootPath={props.onRootPath} />
    </div>
  );
}

type ProgressProps = { scannedDirs?: string; scannedFiles?: string; scannedBytes?: string; currentPath?: string | null };

function progressStats(props: ProgressProps): string {
  if (props.scannedDirs == null) return 'Starting…';
  return `dirs ${props.scannedDirs}, files ${props.scannedFiles}, ${formatBytes({ bytes: props.scannedBytes ?? '0' })}`;
}

export function AnalyzerProgressNotice(props: ProgressProps) {
  return (
    <div role="status" aria-live="polite" className="notice info scanning-pulse analyzer-progress-notice">
      <div className="analyzer-progress-row"><span className="analyzer-progress-label">Analyzing…</span><span className="analyzer-progress-stats num">{progressStats(props)}</span></div>
      {props.currentPath && <div className="analyzer-progress-path-row"><span className="analyzer-progress-path-prefix muted">Current:</span><span className="analyzer-progress-path mono" title={props.currentPath}>{props.currentPath}</span></div>}
      <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
    </div>
  );
}

type StatusCounts = { all: number; safe: number; caution: number; unknown: number; locked: number };

function segClass(active: boolean): string {
  if (active) return 'seg-btn active';
  return 'seg-btn';
}

export function AnalyzerStatusFilters(props: { statusFilter: AnalyzerStatusFilter; counts: StatusCounts; onChange: (v: AnalyzerStatusFilter) => void }) {
  const items: Array<{ key: AnalyzerStatusFilter; label: string; count: number }> = [
    { key: 'all', label: 'All', count: props.counts.all },
    { key: 'safe', label: 'Safe', count: props.counts.safe },
    { key: 'caution', label: 'Caution', count: props.counts.caution },
    { key: 'unknown', label: 'Unknown', count: props.counts.unknown },
    { key: 'locked', label: 'Protected & Read-Only', count: props.counts.locked },
  ];
  return (
    <div className="segmented-toggle scan-status-segmented analyzer-status-segmented analyzer-status-segmented-compact" role="group" aria-label="Analyzer status filters">
      {items.map((it) => {
        const active = props.statusFilter === it.key;
        return <button key={it.key} type="button" className={segClass(active)} aria-pressed={active} onClick={() => props.onChange(it.key)}>{it.label} ({it.count})</button>;
      })}
    </div>
  );
}

type ToolbarProps = {
  statusFilter: AnalyzerStatusFilter;
  statusCounts: StatusCounts;
  query: string;
  sort: 'size' | 'items' | 'name';
  hasResult: boolean;
  analyzing: boolean;
  cleaning: boolean;
  allVisibleExpanded: boolean;
  onStatus: (v: AnalyzerStatusFilter) => void;
  onQuery: (v: string) => void;
  onSort: (v: 'size' | 'items' | 'name') => void;
  onSelectSafe: () => void;
  onToggleExpandCollapse: () => void;
};

function expandCollapseLabel(allVisibleExpanded: boolean): string {
  if (allVisibleExpanded) return 'Collapse All';
  return 'Expand All';
}

function toolbarActionsDisabled(props: ToolbarProps): boolean {
  if (!props.hasResult) return true;
  if (props.analyzing) return true;
  return props.cleaning;
}

export function AnalyzerToolbar(props: ToolbarProps) {
  const disabledActions = toolbarActionsDisabled(props);
  return (
    <div className="analyzer-table-toolbar analyzer-table-toolbar-compact analyzer-toolbar" aria-label="Analyzer table filters">
      <div className="analyzer-toolbar-row analyzer-toolbar-row-top">
        <AnalyzerStatusFilters statusFilter={props.statusFilter} counts={props.statusCounts} onChange={props.onStatus} />
      </div>
      <div className="analyzer-toolbar-row analyzer-toolbar-row-actions">
        <input type="search" className="analyzer-search-input analyzer-search-compact" placeholder="Search folders..." value={props.query} onChange={(e) => props.onQuery(e.target.value)} aria-label="Search analyzer folders" disabled={!props.hasResult} />
        <div className="analyzer-table-toolbar-actions">
          <label className="toolbar-group analyzer-sort-group">Sort<select value={props.sort} onChange={(e) => props.onSort(e.target.value as 'size' | 'items' | 'name')} aria-label="Sort analyzer" disabled={!props.hasResult}><option value="size">Size</option><option value="items">Items</option><option value="name">Name</option></select></label>
          <button type="button" className="btn small" onClick={props.onSelectSafe} disabled={disabledActions} aria-label="Select Safe">Select Safe</button>
          <button type="button" className="btn small subtle-btn" onClick={props.onToggleExpandCollapse} disabled={disabledActions}>{expandCollapseLabel(props.allVisibleExpanded)}</button>
        </div>
      </div>
    </div>
  );
}
