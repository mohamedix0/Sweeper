import { useEffect, useMemo, useState } from 'react';
import {
  formatContractError,
  type CleanProgressEvent,
  type ElevationStatus,
  type ExecuteCleanResponse,
  type ScanResults,
  type Settings,
  type SweeperClient,
} from '../api/client';
import type { ContractError, ScanItemDto, TargetScanResultDto } from '../contract/bindings';
import { formatBytes, normalizeDisplayPath, pluralize, sumDecimalStrings } from '../utils/format';
import {
  InfoTip,
  SESSION_TTL_MS,
  TYPED_CONFIRMATION_PHRASE,
  riskClass,
  type ExplainState,
} from './cleanShared';
import {
  ChevronDownIcon,
  ChevronRightIcon,
  CopyIcon,
  FolderIcon,
  FolderOpenIcon,
  RefreshIcon,
  ShieldIcon,
} from './Icons';

type PreviewResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  issuedAtMs: number;
};

type ScanFilter = 'all' | 'eligible' | 'skipped';
type ScanToolbarState = {
  query: string;
  sort: 'size' | 'name' | 'risk';
  filter: ScanFilter;
};

function isCustomPathTargetId(targetId: string): boolean {
  return targetId === 'custom-path' || targetId.startsWith('custom-path');
}

function isSelectableScanItem(item: { ageEligible: boolean }, target: TargetScanResultDto, allowDeleteUnknown: boolean): boolean {
  if (target.risk === 'Protected') return false;
  if (!item.ageEligible) return false;
  if (target.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return true;
}

function scanSelectionNeedsConfirmation(targets: TargetScanResultDto[], selectedIds: Set<string>): boolean {
  for (const t of targets) {
    for (const item of t.items) {
      if (!selectedIds.has(item.id)) continue;
      if (t.risk === 'Unknown' || isCustomPathTargetId(t.targetId)) return true;
    }
  }
  return false;
}

function humanizeSkippedReason(target: TargetScanResultDto, item: ScanItemDto, allowDeleteUnknown: boolean): string {
  if (target.risk === 'Protected') return 'Protected';
  if (target.risk === 'Unknown' && !allowDeleteUnknown) return 'Skipped · Unknown';
  if (!item.ageEligible) return 'Skipped · < 24h';
  return 'Skipped · Not eligible';
}

function relativeItemName(itemPath: string, resolvedPaths: string[]): string {
  const norm = (p: string) => p.replace(/[\\/]+$/, '').toLowerCase();
  for (const rp of resolvedPaths) {
    const base = norm(rp);
    const low = itemPath.toLowerCase();
    if (low.startsWith(base + '\\') || low.startsWith(base + '/')) {
      return itemPath.slice(rp.replace(/[\\/]+$/, '').length + 1) || itemPath;
    }
  }
  const parts = itemPath.split(/[\\/]/);
  return parts[parts.length - 1] || itemPath;
}

function itemDisplayName(itemPath: string, resolvedPaths: string[]): { name: string; nestedRel: string } {
  const rel = relativeItemName(itemPath, resolvedPaths);
  const parts = rel.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return { name: parts[0] || rel, nestedRel: '' };
  return { name: parts[parts.length - 1] || rel, nestedRel: rel };
}

function resolvedRootPath(target: TargetScanResultDto): string {
  const rp = target.resolvedPaths[0] || '';
  return normalizeDisplayPath(rp);
}

async function copyText(text: string) {
  try { await navigator.clipboard?.writeText(text); } catch { /* ignore */ }
}

function openContainingFolder(path: string) {
  const clean = normalizeDisplayPath(path).replace(/[\\/]+$/, '');
  const idx = clean.lastIndexOf('\\');
  const folder = idx > 2 ? clean.slice(0, idx) : (clean || 'C:\\');
  try { window.open(`file:///${folder.replace(/\\/g, '/')}`, '_blank', 'noopener,noreferrer'); } catch { /* ignore */ }
}

function statusBadgeForScan(target: TargetScanResultDto, item: ScanItemDto | null, selectable: boolean, allowDeleteUnknown: boolean): { label: string; className: string } {
  if (target.risk === 'Protected') return { label: 'Protected', className: 'badge risk-Protected' };
  if (!item) return { label: target.risk, className: riskClass(target.risk) };
  if (!selectable) return { label: humanizeSkippedReason(target, item, allowDeleteUnknown), className: 'badge status-skipped' };
  if (target.risk === 'Caution') return { label: 'Caution', className: riskClass('Caution') };
  if (target.risk === 'Unknown') return { label: 'Unknown', className: riskClass('Unknown') };
  return { label: 'Safe', className: riskClass('Safe') };
}

function itemSafetyLabel(target: TargetScanResultDto, item: ScanItemDto, selectable: boolean, allowDeleteUnknown: boolean): string {
  if (target.risk === 'Protected') return 'Protected';
  if (!selectable) return humanizeSkippedReason(target, item, allowDeleteUnknown);
  return target.risk;
}

function scanGuidance(target: TargetScanResultDto, item: ScanItemDto, selectable: boolean, allowDeleteUnknown: boolean): string {
  if (target.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (target.risk === 'Unknown' && !allowDeleteUnknown) return 'Unknown item is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible.';
  if (target.risk === 'Unknown') return 'Unknown or custom-path item. Review carefully; typed DELETE confirmation is required before cleaning.';
  if (!item.ageEligible) return 'Modified within the minimum age window. Sweeper skips recent files for safety.';
  if (target.targetName === 'User Temp') return 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.';
  if (target.targetName === 'Crash Dumps & WER') return 'Local crash and error reports kept for diagnostics. Removing them does not affect app functionality.';
  if (target.targetName.toLowerCase().includes('npm')) return 'Package manager download cache. It can be redownloaded automatically when needed.';
  if (isCustomPathTargetId(target.targetId)) return 'Custom folder item. Only selected contents are cleaned and protected system directories remain excluded.';
  if (target.risk === 'Caution') return 'Caution category. Review before removal; selected items are permanently deleted.';
  return 'Eligible under current safety settings. Selected items are permanently deleted after review and confirmation.';
}

const RISK_ORDER: Record<string, number> = { Safe: 0, Caution: 1, Unknown: 2, Protected: 3 };

export function ScanAndPreviewView({
  client,
  settings,
  elevationStatus,
  elevationError,
  onOpenLog,
}: {
  client: SweeperClient;
  settings: Settings | null;
  elevationStatus: ElevationStatus;
  elevationError: string | null;
  onOpenLog: (logId: string) => void;
}) {
  const [tier1, setTier1] = useState(true);
  const [tier2, setTier2] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanStatus, setScanStatus] = useState<string>('');
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [scanStartedAt, setScanStartedAt] = useState<number | null>(null);
  const [results, setResults] = useState<ScanResults | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [collapsedTargets, setCollapsedTargets] = useState<Set<string>>(new Set());
  const [toolbar, setToolbar] = useState<ScanToolbarState>({ query: '', sort: 'size', filter: 'all' });
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchInfo, setRelaunchInfo] = useState<string | null>(null);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const [, setTick] = useState(0);

  const targets = useMemo(() => results?.targets ?? [], [results]);
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  useEffect(() => {
    if (!scanStartedAt || !sessionId) return;
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, [scanStartedAt, sessionId]);

  const isSessionExpired = !!scanStartedAt && Date.now() - scanStartedAt > SESSION_TTL_MS;

  // Hide 0 B protected system folders from default scan results (protection logic remains active elsewhere)
  const visibleBaseTargets = useMemo(() => {
    return targets.filter((t) => !(t.risk === 'Protected' && BigInt(t.totalBytes || '0') === 0n));
  }, [targets]);

  const selectedBytes = useMemo(() => {
    const sizes: string[] = [];
    for (const t of visibleBaseTargets) for (const item of t.items) if (selectedIds.has(item.id)) sizes.push(item.size);
    return sumDecimalStrings(sizes);
  }, [visibleBaseTargets, selectedIds]);

  const selectableCount = useMemo(() => {
    let n = 0;
    for (const t of visibleBaseTargets) for (const i of t.items) if (isSelectableScanItem(i, t, allowDeleteUnknown)) n++;
    return n;
  }, [visibleBaseTargets, allowDeleteUnknown]);

  const skippedCount = useMemo(() => {
    let n = 0;
    for (const t of visibleBaseTargets) for (const i of t.items) if (!isSelectableScanItem(i, t, allowDeleteUnknown)) n++;
    return n;
  }, [visibleBaseTargets, allowDeleteUnknown]);

  const totalItemCount = selectableCount + skippedCount;
  const requiresConfirmation = useMemo(() => scanSelectionNeedsConfirmation(visibleBaseTargets, selectedIds), [visibleBaseTargets, selectedIds]);

  const eligibleSafeIds = useMemo(() => {
    const ids: string[] = [];
    for (const t of visibleBaseTargets) {
      if (t.risk !== 'Safe') continue;
      for (const i of t.items) if (isSelectableScanItem(i, t, allowDeleteUnknown)) ids.push(i.id);
    }
    return ids;
  }, [visibleBaseTargets, allowDeleteUnknown]);

  const heroReclaimable = useMemo(() => {
    let total = 0n;
    for (const t of visibleBaseTargets) total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [visibleBaseTargets]);
  const tier1Reclaimable = useMemo(() => {
    let total = 0n;
    for (const t of visibleBaseTargets) if (t.tier === 'Tier1') total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [visibleBaseTargets]);
  const tier2Reclaimable = useMemo(() => {
    let total = 0n;
    for (const t of visibleBaseTargets) if (t.tier === 'Tier2') total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [visibleBaseTargets]);

  function resetForInvalidSession(message?: string) {
    setSessionId(null);
    setScanStartedAt(null);
    setResults(null);
    setSelectedIds(new Set());
    setCollapsedTargets(new Set());
    setPreview(null);
    setConfirmText('');
    setCleanProgress(null);
    setInspected(null);
    if (message) setInfo(message);
  }

  async function inspectItem(itemId: string) {
    try { setInspected(await client.explainItem(itemId)); } catch { /* non-fatal */ }
  }

  async function runScan() {
    setError(null); setInfo(null); setPreview(null); setInspected(null); setSelectedIds(new Set());
    setCollapsedTargets(new Set()); setResults(null); setCleanSummary(null); setCleanProgress(null); setScanning(true);
    setScanStatus('Starting Scan…');
    const tiers = [tier1 ? 'Tier1' : null, tier2 ? 'Tier2' : null].filter(Boolean) as ('Tier1' | 'Tier2')[];
    try {
      const { sessionId: sid } = await client.startScan(tiers.length ? tiers : null, (ev) => {
        if (ev.kind === 'started') setScanStatus('Scan in progress…');
        if (ev.kind === 'completed') setScanStatus('');
        if (ev.kind === 'failed') setScanStatus('Scan failed.');
      });
      setSessionId(sid);
      setScanStartedAt(Date.now());
      const data = await client.getScanResults(sid);
      setResults(data);
      const auto = new Set<string>();
      for (const t of data.targets) {
        if (t.risk !== 'Safe') continue;
        for (const i of t.items) if (isSelectableScanItem(i, t, settings?.allowDeleteUnknown ?? false)) auto.add(i.id);
      }
      setSelectedIds(auto);

      const baseVisible = data.targets.filter((t) => !(t.risk === 'Protected' && BigInt(t.totalBytes || '0') === 0n));
      const collapsed = new Set<string>();
      const firstActive = baseVisible.find((t) => t.targetName === 'User Temp') ?? baseVisible[0] ?? null;
      for (const t of baseVisible) {
        if (firstActive && t.targetId === firstActive.targetId) continue;
        collapsed.add(t.targetId);
      }
      setCollapsedTargets(collapsed);

      const firstTarget = firstActive ?? baseVisible[0];
      const firstItem = firstTarget?.items[0] ?? data.targets.flatMap((t) => t.items)[0];
      if (firstItem) {
        try { setInspected(await client.explainItem(firstItem.id)); } catch { /* ignore */ }
      }
      setScanStatus('');
    } catch (e) {
      setError(formatContractError(e));
      const ce = e as ContractError | undefined;
      if (ce?.code === 'sessionInvalid') resetForInvalidSession();
    } finally {
      setScanning(false);
    }
  }

  function toggleItem(item: ScanItemDto, target: TargetScanResultDto) {
    if (!isSelectableScanItem(item, target, allowDeleteUnknown)) { void inspectItem(item.id); return; }
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => { const next = new Set(prev); if (next.has(item.id)) next.delete(item.id); else next.add(item.id); return next; });
    void inspectItem(item.id);
  }

  function setTargetSelection(target: TargetScanResultDto, select: boolean) {
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const item of target.items) {
        if (!isSelectableScanItem(item, target, allowDeleteUnknown)) continue;
        if (select) next.add(item.id); else next.delete(item.id);
      }
      return next;
    });
  }

  function selectAllSafe() {
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const id of eligibleSafeIds) next.add(id);
      return next;
    });
  }

  function clearSelection() {
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds(new Set());
  }

  async function runPreview() {
    if (!sessionId) return;
    if (isSessionExpired) {
      setError('Scan session expired or was invalidated. Start a new Scan.');
      resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
      return;
    }
    setError(null); setInfo(null); setPreviewLoading(true); setCleanSummary(null); setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(sessionId, Array.from(selectedIds), confirmations);
      setPreview({ ...res, issuedAtMs: Date.now() });
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') setPreview(null);
    } finally { setPreviewLoading(false); }
  }

  async function runClean() {
    if (!preview) return;
    setError(null); setInfo(null); setCleaning(true); setCleanProgress(null); setCleanSummary(null);
    try {
      const res = await client.executeClean(preview.previewToken, (ev) => setCleanProgress(ev));
      setCleanSummary(res);
      setPreview(null);
      setSelectedIds(new Set());
      setConfirmText('');
      setInfo('Clean completed. Review the summary and Activity History below.');
      if (sessionId) { try { setResults(await client.getScanResults(sessionId)); } catch { /* ignore */ } }
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Preview again.'); }
    } finally { setCleaning(false); }
  }

  async function runAsAdministrator() {
    setRelaunching(true);
    setRelaunchInfo(null);
    setRelaunchError(null);
    try {
      const res = await client.relaunchElevated();
      if (res.launched) setRelaunchInfo('Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.');
      else setRelaunchError('Administrator launch was not started.');
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally { setRelaunching(false); }
  }

  function toggleCollapsed(targetId: string) {
    setCollapsedTargets((prev) => { const next = new Set(prev); if (next.has(targetId)) next.delete(targetId); else next.add(targetId); return next; });
  }

  const allCollapsed = visibleBaseTargets.length > 0 && collapsedTargets.size === visibleBaseTargets.length;
  function toggleExpandCollapseAll() {
    if (allCollapsed) setCollapsedTargets(new Set());
    else setCollapsedTargets(new Set(visibleBaseTargets.map((t) => t.targetId)));
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading && !cleaning && !isSessionExpired && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !scanning && !previewLoading && !cleaning;

  const q = toolbar.query.trim().toLowerCase();
  const visibleTargets = useMemo(() => {
    return visibleBaseTargets.map((t) => {
      let items = t.items.filter((i) => {
        const selectable = isSelectableScanItem(i, t, allowDeleteUnknown);
        if (toolbar.filter === 'eligible' && !selectable) return false;
        if (toolbar.filter === 'skipped' && selectable) return false;
        if (q) {
          const rel = relativeItemName(i.path, t.resolvedPaths).toLowerCase();
          if (!i.path.toLowerCase().includes(q) && !rel.includes(q)) return false;
        }
        return true;
      });
      items = [...items].sort((a, b) => {
        if (toolbar.sort === 'name') return relativeItemName(a.path, t.resolvedPaths).localeCompare(relativeItemName(b.path, t.resolvedPaths));
        if (toolbar.sort === 'risk') {
          const rr = (RISK_ORDER[t.risk] ?? 9) - (RISK_ORDER[t.risk] ?? 9);
          if (rr !== 0) return rr;
        }
        const da = BigInt(a.size), db = BigInt(b.size);
        return da === db ? 0 : (da > db ? -1 : 1);
      });
      return { target: t, items };
    }).filter((g) => g.items.length > 0 || (!q && toolbar.filter === 'all'));
  }, [visibleBaseTargets, toolbar, allowDeleteUnknown, q]);

  const hasResults = !!results;
  const allSafeSelected = eligibleSafeIds.length > 0 && eligibleSafeIds.every((id) => selectedIds.has(id));

  const inspectedCtx = useMemo(() => {
    if (!inspected) return null;
    for (const t of visibleBaseTargets) {
      const it = t.items.find((i) => i.id === inspected.itemId);
      if (it) return { target: t, item: it };
    }
    return null;
  }, [inspected, visibleBaseTargets]);

  return (
    <div className="view-stack">
      <header className="page-header scan-header-row" aria-label="Scan and Clean header">
        <div className="page-header-left">
          <h1 className="page-title" id="scan-heading">Scan &amp; Clean</h1>
          {results && <span className="hero-metric-badge num" aria-label="Reclaimable total">{formatBytes(heroReclaimable)} reclaimable</span>}
        </div>
        <div className="page-header-right scan-header-actions">
          <button
            type="button"
            className={`btn small scope-admin-compact ${!elevationStatus.isElevated ? 'locked' : 'ready'}`}
            onClick={!elevationStatus.isElevated ? runAsAdministrator : undefined}
            disabled={relaunching || elevationStatus.isElevated}
            aria-label={elevationStatus.isElevated ? 'Standard Mode elevated' : 'Standard Mode elevate'}
            title={elevationStatus.isElevated ? 'System-wide cleanup unlocked' : 'System-Wide Cleanup Locked · Requires Administrator'}
          >
            <ShieldIcon aria-hidden="true" />
            <span>{elevationStatus.isElevated ? 'System-wide cleanup unlocked' : 'Standard Mode · Elevate'}</span>
          </button>
          <button
            key={hasResults ? 'rescan' : 'scan'}
            className={hasResults ? 'btn secondary-action' : 'btn primary'}
            onClick={runScan}
            disabled={scanning || cleaning}
            aria-label="Start Scan"
          >
            {hasResults ? <><RefreshIcon aria-hidden="true" /> <span>Rescan</span></> : 'Start Scan'}
          </button>
        </div>
      </header>

      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {relaunchInfo && <div className="notice info" role="status">{relaunchInfo}</div>}
      {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}
      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      <section className="scan-filter-bar" aria-label="Scan filters">
        <div className="scan-filter-chips">
          <label className={`chip-toggle ${tier1 ? 'active' : ''}`}>
            <input type="checkbox" checked={tier1} onChange={(e) => setTier1(e.target.checked)} disabled={scanning || cleaning} aria-label="System & App Temp" />
            <span>Temp {results ? formatBytes(tier1Reclaimable) : '6 MB'}</span>
          </label>
          <InfoTip label="About System &amp; App Temp (Tier 1)">
            <p><strong>System &amp; App Temp (Tier 1)</strong><br />Windows Temp, crash dumps &amp; app caches older than 24h.</p>
          </InfoTip>
          <label className={`chip-toggle ${tier2 ? 'active' : ''}`}>
            <input type="checkbox" checked={tier2} onChange={(e) => setTier2(e.target.checked)} disabled={scanning || cleaning} aria-label="Developer Caches" />
            <span>Dev Caches {results ? formatBytes(tier2Reclaimable) : '7 MB'}</span>
          </label>
          <InfoTip label="About Developer Caches (Tier 2)">
            <p><strong>Developer Caches (Tier 2)</strong><br />npm, Cargo, pip &amp; build caches that rebuild automatically.</p>
          </InfoTip>
        </div>
        <input type="search" className="scan-search-input" placeholder="Search files or paths..." value={toolbar.query} onChange={(e) => setToolbar({ ...toolbar, query: e.target.value })} aria-label="Search files or paths" />
        <label className="toolbar-group">Sort
          <select value={toolbar.sort} onChange={(e) => setToolbar({ ...toolbar, sort: e.target.value as ScanToolbarState['sort'] })} aria-label="Sort scan results">
            <option value="size">Size ↓</option>
            <option value="name">Name</option>
            <option value="risk">Risk</option>
          </select>
        </label>
        <div className="toolbar-chip-group" role="group" aria-label="Status filters">
          <button type="button" className={`chip-btn ${toolbar.filter === 'all' ? 'active' : ''}`} onClick={() => setToolbar({ ...toolbar, filter: 'all' })}>All ({totalItemCount})</button>
          <button type="button" className={`chip-btn ${toolbar.filter === 'eligible' ? 'active' : ''}`} onClick={() => setToolbar({ ...toolbar, filter: 'eligible' })}>Eligible ({selectableCount})</button>
          <button type="button" className={`chip-btn ${toolbar.filter === 'skipped' ? 'active' : ''}`} onClick={() => setToolbar({ ...toolbar, filter: 'skipped' })}>Skipped ({skippedCount})</button>
        </div>
        <button className="btn small subtle-btn" onClick={toggleExpandCollapseAll} disabled={cleaning || !results}>{allCollapsed ? 'Expand All' : 'Collapse All'}</button>
        <button className="btn small" onClick={selectAllSafe} disabled={cleaning || eligibleSafeIds.length === 0}>Select Safe</button>
      </section>

      {scanning && (
        <div role="status" aria-live="polite" className="notice info scanning-pulse">
          {scanStatus || 'Scanning…'}
          <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
        </div>
      )}
      {!scanning && scanStatus && <div className={`notice ${scanStatus.toLowerCase().includes('failed') ? 'error' : ''}`} role={scanStatus.toLowerCase().includes('failed') ? 'alert' : 'status'}>{scanStatus}</div>}

      {!results && !scanning && (
        <section className="empty-state" aria-label="Scan onboarding">
          <h3>Ready to scan</h3>
          <p>Choose scope above, then run <strong>Start Scan</strong>. Nothing is removed until you Preview and confirm <strong>Clean Permanently</strong>.</p>
        </section>
      )}

      {scanning && !results && (
        <section className="master-detail scan-layout" aria-label="Scanning">
          <div className="master-pane">
            <div className="scan-table-header" aria-hidden="true"><span>Item</span><span>Status</span><span className="num">Size</span></div>
            <div className="pane-body flush" aria-hidden="true">
              <div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" />
            </div>
          </div>
          <div className="detail-pane selection-pane scan-inspector-pane">
            <div className="pane-header"><h3>Inspector</h3></div>
            <div className="detail-scroll"><p className="muted">Scanning for reclaimable space…</p></div>
          </div>
        </section>
      )}

      {results && (
        <>
          <section className="master-detail scan-layout scan-layout-with-bar" aria-label="Scan results and review">
            <div className="master-pane scan-master-pane">
              <div className="scan-table-header" aria-hidden="true">
                <span>Item</span>
                <span>Status</span>
                <span className="num">Size</span>
              </div>
              <div className="pane-body flush scan-results-body">
                {visibleBaseTargets.length === 0 && <div className="empty-state"><h3>No targets</h3><p>No Targets returned for this Scan.</p></div>}
                <div className="target-list scan-grid-list">
                  {visibleTargets.map(({ target: t, items: visibleItems }) => {
                    const selectableItems = t.items.filter((i) => isSelectableScanItem(i, t, allowDeleteUnknown));
                    const selectableIds = selectableItems.map((i) => i.id);
                    const selectedInTarget = selectableIds.filter((id) => selectedIds.has(id)).length;
                    const allSelected = selectableIds.length > 0 && selectedInTarget === selectableIds.length;
                    const someSelected = selectedInTarget > 0 && !allSelected;
                    const collapsed = collapsedTargets.has(t.targetId);
                    const rootPath = resolvedRootPath(t);
                    const firstItem = t.items[0];
                    const groupStatus = t.risk === 'Protected' ? { label: 'Protected', className: 'badge risk-Protected' } : (t.risk === 'Caution' ? { label: 'Caution', className: 'badge risk-Caution' } : null);
                    return (
                      <article key={t.targetId} className="target-card" aria-labelledby={`target-${t.targetId}-name`}>
                        <div
                          className={`scan-row scan-group-row ${inspected?.itemId === firstItem?.id ? 'inspected' : ''}`}
                          role="button"
                          tabIndex={0}
                          onClick={() => { if (firstItem) void inspectItem(firstItem.id); }}
                          onKeyDown={(e) => { if ((e.key === 'Enter' || e.key === ' ') && firstItem) { e.preventDefault(); void inspectItem(firstItem.id); } }}
                        >
                          <div className="scan-col-main">
                            <button type="button" className="target-collapse-btn" onClick={(e) => { e.stopPropagation(); toggleCollapsed(t.targetId); }} aria-label={collapsed ? `Expand ${t.targetName}` : `Collapse ${t.targetName}`}>
                              {collapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}
                            </button>
                            <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
                              {t.risk === 'Protected' ? (
                                <span className="lock-slot" aria-label={`${t.targetName} protected`} title="Protected"><ShieldIcon /></span>
                              ) : (
                                <label className="checkbox target-group-check">
                                  <input
                                    type="checkbox"
                                    aria-label={`Select all eligible items in ${t.targetName}`}
                                    checked={allSelected}
                                    ref={(el) => { if (el) el.indeterminate = someSelected; }}
                                    disabled={cleaning || selectableIds.length === 0}
                                    onChange={(e) => setTargetSelection(t, e.target.checked)}
                                  />
                                </label>
                              )}
                            </span>
                            <FolderIcon className="row-icon" aria-hidden="true" />
                            <div className="scan-main-text scan-group-main-text">
                              <div className="scan-title-line">
                                <span className="target-name-btn scan-group-title" id={`target-${t.targetId}-name`}>{t.targetName}</span>
                              </div>
                              <div className="scan-subpath">{selectedInTarget} of {t.eligibleCount} eligible · {pluralize(Number(t.fileCount || '0'), 'file', 'files')}{rootPath ? ` · ${rootPath}` : ''}</div>
                            </div>
                          </div>
                          <div className="scan-col-status scan-group-status-col">
                            {groupStatus ? <span className={groupStatus.className}>{groupStatus.label}</span> : <span className="status-muted-inline" />}
                          </div>
                          <div className="scan-col-size scan-group-size-col num">
                            <div className="scan-group-size-primary">{formatBytes(t.eligibleBytes)}</div>
                          </div>
                        </div>
                        {!collapsed && (
                          <div className="items" role="group" aria-label={`${t.targetName} items`}>
                            {t.items.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items.</span></div>}
                            {t.items.length > 0 && visibleItems.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items shown for current filters.</span></div>}
                            {visibleItems.map((item) => {
                              const selectable = isSelectableScanItem(item, t, allowDeleteUnknown);
                              const checked = selectedIds.has(item.id);
                              const { name, nestedRel } = itemDisplayName(item.path, t.resolvedPaths);
                              const isProtected = t.risk === 'Protected';
                              const inspectedMatch = inspected?.itemId === item.id;
                              const status = statusBadgeForScan(t, item, selectable, allowDeleteUnknown);
                              return (
                                <div
                                  key={item.id}
                                  className={`scan-row scan-item-row ${inspectedMatch ? 'inspected' : ''}`}
                                  role="button"
                                  tabIndex={0}
                                  onClick={() => toggleItem(item, t)}
                                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleItem(item, t); } }}
                                >
                                  <div className="scan-col-main">
                                    <span className="row-indent" aria-hidden="true" />
                                    <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
                                      {isProtected ? (
                                        <span className="lock-slot" aria-label={`${normalizeDisplayPath(item.path)} protected`} title="Protected"><ShieldIcon /></span>
                                      ) : (
                                        <label className="checkbox">
                                          <input type="checkbox" aria-label={`Select ${normalizeDisplayPath(item.path)}`} checked={checked} disabled={!selectable || cleaning} onChange={() => toggleItem(item, t)} />
                                        </label>
                                      )}
                                    </span>
                                    <div className="scan-main-text">
                                      <div className="scan-title-line">
                                        <span className="item-name" title={normalizeDisplayPath(item.path)}>{name}</span>
                                        {nestedRel && nestedRel !== name ? <span className="scan-subpath-inline mono" title={nestedRel}>{nestedRel}</span> : null}
                                      </div>
                                    </div>
                                  </div>
                                  <div className="scan-col-status">
                                    <span className={status.className} aria-label={status.label.startsWith('Skipped') ? 'Status Skipped' : `Status ${status.label}`}>{status.label}</span>
                                  </div>
                                  <div className="scan-col-size num">{formatBytes(item.size)}</div>
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </article>
                    );
                  })}
                </div>
              </div>
            </div>

            <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="inspector-heading">
              <div className="pane-header">
                <h3 id="inspector-heading">Inspector</h3>
              </div>
              <div className="detail-scroll">
                <div className="inspector-section" aria-label="Item inspector">
                  {inspected && inspectedCtx ? (() => {
                    const { item, target } = inspectedCtx;
                    const selectable = isSelectableScanItem(item, target, allowDeleteUnknown);
                    const fullPath = normalizeDisplayPath(item.path);
                    const itemTitle = relativeItemName(item.path, target.resolvedPaths).split(/[\\/]/).pop() || inspected.title;
                    const status = statusBadgeForScan(target, item, selectable, allowDeleteUnknown);
                    const eligibility = selectable ? 'Eligible' : humanizeSkippedReason(target, item, allowDeleteUnknown);
                    const ageLabel = item.ageEligible ? 'Eligible age' : 'Recent (< 24h)';
                    return (
                      <>
                        <div className="inspector-header-block">
                          <div className="inspector-title-row">
                            <FolderIcon className="inspector-leading-icon" aria-hidden="true" />
                            <div className="inspector-title-copy">
                              <span className="inspector-headline-name" title={fullPath}>{itemTitle}</span>
                              <span className="inspector-subtitle muted">{target.targetName}</span>
                            </div>
                            <span className={status.className}>{status.label}</span>
                          </div>
                        </div>
                        <dl className="inspector-meta-grid" aria-label="Item metadata">
                          <div><dt>Size</dt><dd className="num">{formatBytes(item.size)}</dd></div>
                          <div><dt>Modified / Age</dt><dd>{ageLabel}</dd></div>
                          <div><dt>Safety</dt><dd>{itemSafetyLabel(target, item, selectable, allowDeleteUnknown)}</dd></div>
                          <div><dt>Eligibility</dt><dd>{eligibility}</dd></div>
                        </dl>
                        <p className="inspector-guidance">{scanGuidance(target, item, selectable, allowDeleteUnknown)}</p>
                        <div className="path-strip" aria-label="Full path">
                          <span className="mono path-strip-text" title={fullPath}>{fullPath || 'C:\\'}</span>
                          <button type="button" className="icon-only-btn subtle path-strip-btn" onClick={() => void copyText(fullPath)} aria-label="Copy path" title="Copy path"><CopyIcon /></button>
                          <button type="button" className="icon-only-btn subtle path-strip-btn" onClick={() => openContainingFolder(fullPath)} aria-label="Open Folder" title="Open Folder"><FolderOpenIcon /></button>
                        </div>
                      </>
                    );
                  })() : (
                    <p className="inspector-empty">Select an item to see its purpose, risk, full path, and eligibility.</p>
                  )}
                </div>

                {requiresConfirmation && (
                  <div className="notice warn" role="note">
                    <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path items). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
                    <div className="field">
                      <label htmlFor="confirm-delete">Typed confirmation</label>
                      <input id="confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={cleaning} />
                    </div>
                  </div>
                )}

                {preview && (
                  <div aria-live="polite" className="preview-breakdown">
                    <h4 className="preview-breakdown-title">Preview Breakdown</h4>
                    <p className="preview-total"><strong>Total selected:</strong> {pluralize(Number(preview.totalSelectedCount || '0'), 'item', 'items')}, <span className="num">{formatBytes(preview.totalSelectedBytes)}</span></p>
                    <table className="table" aria-label="Preview breakdown by Target">
                      <thead><tr><th>Target</th><th>Count</th><th>Size</th></tr></thead>
                      <tbody>
                        {preview.breakdown.map((b) => (
                          <tr key={b.targetId}><td>{b.targetName}</td><td>{b.selectedCount}</td><td>{formatBytes(b.selectedBytes)}</td></tr>
                        ))}
                      </tbody>
                    </table>
                    <div className="clean-footer" role="alert" aria-live="polite">
                      <div className="clean-footer-warning"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div>
                    </div>
                    {cleaning && (
                      <div className="notice info" role="status" aria-live="polite">
                        <div>Cleaning in progress…</div>
                        {cleanProgress && <div>Progress: {cleanProgress.completedItems} / {cleanProgress.totalItems} items — {formatBytes(cleanProgress.bytesFreedSoFar)} freed so far ({cleanProgress.status})</div>}
                        <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
                      </div>
                    )}
                  </div>
                )}

                {cleanSummary && (
                  <div className="clean-summary-section" aria-labelledby="clean-summary-heading" role="status">
                    <h4 id="clean-summary-heading" className="clean-summary-title">Clean Summary</h4>
                    {cleanSummary.aborted && <div className="notice error" role="alert"><strong>Clean aborted.</strong> {cleanSummary.abortReason || 'The operation was aborted.'}</div>}
                    <dl className="kv">
                      <dt>Bytes freed</dt><dd className="num">{formatBytes(cleanSummary.bytesFreed)}</dd>
                      <dt>Deleted count</dt><dd className="num">{cleanSummary.deletedCount}</dd>
                      <dt>Skipped count</dt><dd className="num">{cleanSummary.skippedCount}</dd>
                    </dl>
                    <div>
                      <strong className="subheading-xs">Skipped reasons</strong>
                      {cleanSummary.skippedReasons.length === 0 ? <p className="muted small-note" aria-label="Skipped reasons">None.</p> : (
                        <ul aria-label="Skipped reasons" className="skipped-reasons">{cleanSummary.skippedReasons.map((r, idx) => <li key={`${r.reason}-${idx}`}>{r.reason} — {r.count}</li>)}</ul>
                      )}
                    </div>
                    {cleanSummary.logId && <div className="row row-top"><button className="btn" onClick={() => onOpenLog(cleanSummary.logId!)}>View Operation Log</button></div>}
                  </div>
                )}
              </div>
            </aside>
          </section>

          <div className="scan-bottom-bar" aria-label="Cleanup actions">
            <div className="scan-bottom-bar-left">
              <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(selectedIds.size, 'item', 'items')} selected</strong> · <span className="num">{formatBytes(selectedBytes)}</span> reclaimable</span>
              {selectedIds.size > 0 && <button type="button" className="btn small subtle-btn" onClick={clearSelection} disabled={cleaning}>Clear</button>}
            </div>
            <div className="scan-bottom-bar-right">
              {!preview ? (
                <button className="btn primary" onClick={runPreview} disabled={!canPreview} aria-label="Review & Clean">{previewLoading ? 'Generating Preview…' : `Review & Clean${selectedIds.size > 0 ? ` (${formatBytes(selectedBytes)})` : ''}`}</button>
              ) : (
                <button className="btn danger primary" onClick={runClean} disabled={!canClean} aria-label="Clean Permanently">{cleaning ? 'Cleaning…' : 'Clean Permanently'}</button>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
