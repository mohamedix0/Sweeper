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
  ExplainDrawer,
  InfoTip,
  SESSION_TTL_MS,
  TYPED_CONFIRMATION_PHRASE,
  riskClass,
  type ExplainState,
} from './cleanShared';
import { ChevronDownIcon, ChevronRightIcon, CopyIcon, LockIcon, RefreshIcon, ShieldIcon } from './Icons';

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

async function copyText(text: string) {
  try { await navigator.clipboard?.writeText(text); } catch { /* ignore */ }
}

function resolvedRootPath(target: TargetScanResultDto): string {
  const rp = target.resolvedPaths[0] || '';
  return normalizeDisplayPath(rp);
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
  const [drawerExplain, setDrawerExplain] = useState<ExplainState>(null);
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

  const selectedBytes = useMemo(() => {
    const sizes: string[] = [];
    for (const t of targets) for (const item of t.items) if (selectedIds.has(item.id)) sizes.push(item.size);
    return sumDecimalStrings(sizes);
  }, [targets, selectedIds]);

  const selectableCount = useMemo(() => {
    let n = 0;
    for (const t of targets) for (const i of t.items) if (isSelectableScanItem(i, t, allowDeleteUnknown)) n++;
    return n;
  }, [targets, allowDeleteUnknown]);

  const skippedCount = useMemo(() => {
    let n = 0;
    for (const t of targets) for (const i of t.items) if (!isSelectableScanItem(i, t, allowDeleteUnknown)) n++;
    return n;
  }, [targets, allowDeleteUnknown]);

  const totalItemCount = selectableCount + skippedCount;

  const requiresConfirmation = useMemo(() => scanSelectionNeedsConfirmation(targets, selectedIds), [targets, selectedIds]);

  const eligibleSafeIds = useMemo(() => {
    const ids: string[] = [];
    for (const t of targets) {
      if (t.risk !== 'Safe') continue;
      for (const i of t.items) if (isSelectableScanItem(i, t, allowDeleteUnknown)) ids.push(i.id);
    }
    return ids;
  }, [targets, allowDeleteUnknown]);

  const heroReclaimable = useMemo(() => {
    let total = 0n;
    for (const t of targets) total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [targets]);
  const tier1Reclaimable = useMemo(() => {
    let total = 0n;
    for (const t of targets) if (t.tier === 'Tier1') total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [targets]);
  const tier2Reclaimable = useMemo(() => {
    let total = 0n;
    for (const t of targets) if (t.tier === 'Tier2') total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [targets]);

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
    setError(null); setInfo(null); setPreview(null); setDrawerExplain(null); setInspected(null); setSelectedIds(new Set());
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
      const collapsed = new Set<string>();
      for (const t of data.targets) {
        if (t.risk === 'Protected') collapsed.add(t.targetId);
      }
      setCollapsedTargets(collapsed);
      const firstItem = data.targets.flatMap((t) => t.items)[0];
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
    if (!isSelectableScanItem(item, target, allowDeleteUnknown)) return;
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

  async function onExplain(itemId: string) {
    setError(null);
    try {
      const ex = await client.explainItem(itemId);
      setDrawerExplain(ex);
      setInspected(ex);
    } catch (e) { setError(formatContractError(e)); }
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
      if (res.launched) {
        setRelaunchInfo('Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.');
      } else {
        setRelaunchError('Administrator launch was not started.');
      }
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  }

  function toggleCollapsed(targetId: string) {
    setCollapsedTargets((prev) => { const next = new Set(prev); if (next.has(targetId)) next.delete(targetId); else next.add(targetId); return next; });
  }

  const allCollapsed = targets.length > 0 && collapsedTargets.size === targets.length;
  function toggleExpandCollapseAll() {
    if (allCollapsed) setCollapsedTargets(new Set());
    else setCollapsedTargets(new Set(targets.map((t) => t.targetId)));
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading && !cleaning && !isSessionExpired && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !scanning && !previewLoading && !cleaning;

  const q = toolbar.query.trim().toLowerCase();
  const visibleTargets = useMemo(() => {
    return targets.map((t) => {
      let items = t.items.filter((i) => {
        const selectable = isSelectableScanItem(i, t, allowDeleteUnknown);
        if (toolbar.filter === 'eligible' && !selectable) return false;
        if (toolbar.filter === 'skipped' && selectable) return false;
        if (q && !i.path.toLowerCase().includes(q) && !relativeItemName(i.path, t.resolvedPaths).toLowerCase().includes(q)) return false;
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
  }, [targets, toolbar, allowDeleteUnknown, q]);

  const selectedBreakdown = useMemo(() => {
    const by = new Map<string, { name: string; count: number; bytes: bigint }>();
    for (const t of targets) {
      for (const i of t.items) {
        if (!selectedIds.has(i.id)) continue;
        const e = by.get(t.targetId) ?? { name: t.targetName, count: 0, bytes: 0n };
        e.count += 1; e.bytes += BigInt(i.size || '0'); by.set(t.targetId, e);
      }
    }
    return Array.from(by.values());
  }, [targets, selectedIds]);

  const hasResults = !!results;

  return (
    <div className="view-stack">
      <header className="page-header" aria-label="Scan and Clean header">
        <div className="page-header-left">
          <h1 className="page-title" id="scan-heading">Scan &amp; Clean</h1>
          {results && <span className="hero-metric-badge num" aria-label="Reclaimable total">{formatBytes(heroReclaimable)} reclaimable · {pluralize(selectableCount, 'eligible item', 'eligible items')}</span>}
        </div>
        <div className="page-header-right">
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

      <section className="scope-bar" aria-label="Scan scope">
        <div className={`scope-segment ${tier1 ? 'active' : ''}`}>
          <label className="scope-check">
            <input type="checkbox" checked={tier1} onChange={(e) => setTier1(e.target.checked)} disabled={scanning || cleaning} aria-label="System & App Temp" />
            <span className="scope-segment-label">System &amp; App Temp</span>
          </label>
          <span className="scope-segment-meta num">{results ? formatBytes(tier1Reclaimable) : '6 MB'}</span>
          <InfoTip label="About System &amp; App Temp (Tier 1)">
            <p><strong>System &amp; App Temp (Tier 1)</strong><br />Windows Temp, crash dumps &amp; app caches older than 24h.</p>
          </InfoTip>
        </div>
        <span className="scope-divider" aria-hidden="true" />
        <div className={`scope-segment ${tier2 ? 'active' : ''}`}>
          <label className="scope-check">
            <input type="checkbox" checked={tier2} onChange={(e) => setTier2(e.target.checked)} disabled={scanning || cleaning} aria-label="Developer Caches" />
            <span className="scope-segment-label">Developer Caches</span>
          </label>
          <span className="scope-segment-meta num">{results ? formatBytes(tier2Reclaimable) : '7 MB'}</span>
          <InfoTip label="About Developer Caches (Tier 2)">
            <p><strong>Developer Caches (Tier 2)</strong><br />npm, Cargo, pip &amp; build caches that rebuild automatically.</p>
          </InfoTip>
        </div>
        <div className={`scope-admin ${!elevationStatus.isElevated ? 'locked' : 'ready scope-admin-clean'}`} aria-label={elevationStatus.isElevated ? 'Admin cleanup ready' : 'System-Wide Cleanup Locked · Requires Administrator'}>
          <ShieldIcon />
          <span className="scope-admin-text">{elevationStatus.isElevated ? 'System-wide cleanup unlocked' : 'System-Wide Cleanup Locked · Requires Administrator'}</span>
          {!elevationStatus.isElevated && <button className="btn small" onClick={runAsAdministrator} disabled={relaunching}>Run as administrator</button>}
        </div>
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
            <div className="pane-header"><h2>Scan Results</h2></div>
            <div className="pane-body flush" aria-hidden="true">
              <div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" />
            </div>
          </div>
          <div className="detail-pane selection-pane">
            <div className="pane-header"><h3 id="preview-heading">Inspector</h3></div>
            <div className="detail-scroll"><p className="muted">Scanning for reclaimable space…</p></div>
          </div>
        </section>
      )}

      {results && (
        <>
          <section className="master-detail scan-layout scan-layout-with-bar" aria-label="Scan results and review">
            <div className="master-pane scan-master-pane">
              <div className="scan-toolbar" role="toolbar" aria-label="Scan results toolbar">
                <input type="search" placeholder="Search files or paths..." value={toolbar.query} onChange={(e) => setToolbar({ ...toolbar, query: e.target.value })} aria-label="Search files or paths" />
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
                <div className="toolbar-spacer" />
                <button className="btn small subtle-btn" onClick={toggleExpandCollapseAll} disabled={cleaning}>{allCollapsed ? 'Expand All' : 'Collapse All'}</button>
                <button className="btn small" onClick={selectAllSafe} disabled={cleaning || eligibleSafeIds.length === 0}>Select All Safe</button>
              </div>
              <div className="pane-header scan-results-header">
                <div className="pane-header-left">
                  <h2 id="results-heading">Scan Results</h2>
                  <span className="pane-meta-pill num">{pluralize(selectableCount, 'eligible item', 'eligible items')}</span>
                </div>
              </div>
              <div className="pane-body flush scan-results-body">
                {targets.length === 0 && <div className="empty-state"><h3>No targets</h3><p>No Targets returned for this Scan.</p></div>}
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
                    return (
                      <article key={t.targetId} className="target-card" aria-labelledby={`target-${t.targetId}-name`}>
                        <div
                          className="scan-row scan-group-row"
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
                                <span className="lock-slot" aria-label={`${t.targetName} protected`} title="Protected">
                                  <LockIcon />
                                </span>
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
                            <div className="scan-main-text scan-group-main-text">
                              <div className="scan-title-line">
                                <span className="target-name-btn scan-group-title" id={`target-${t.targetId}-name`}>{t.targetName}</span>
                              </div>
                              {rootPath && <div className="scan-subpath" title={rootPath}>{rootPath}</div>}
                            </div>
                          </div>
                          <div className="scan-col-status scan-group-status-col">
                            {t.risk === 'Protected' ? <span className="inline-tag scan-group-protected-badge">Protected</span> : null}
                          </div>
                          <div className="scan-col-size scan-group-size-col num">
                            <div className="scan-group-size-primary">{formatBytes(t.eligibleBytes)}</div>
                            <div className="scan-group-size-secondary">{selectedInTarget} of {t.eligibleCount} eligible · {pluralize(Number(t.fileCount || '0'), 'file', 'files')}</div>
                          </div>
                        </div>
                        {!collapsed && (
                          <div className="items" role="group" aria-label={`${t.targetName} items`}>
                            {t.items.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items.</span></div>}
                            {t.items.length > 0 && visibleItems.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items shown for current filters.</span></div>}
                            {visibleItems.map((item) => {
                              const selectable = isSelectableScanItem(item, t, allowDeleteUnknown);
                              const checked = selectedIds.has(item.id);
                              const name = relativeItemName(item.path, t.resolvedPaths);
                              const isProtected = t.risk === 'Protected';
                              const isSkipped = !selectable && !isProtected;
                              const inspectedMatch = inspected?.itemId === item.id;
                              const statusLabel = isProtected ? 'Protected' : (isSkipped ? humanizeSkippedReason(t, item, allowDeleteUnknown) : (t.risk === 'Caution' || t.risk === 'Unknown' ? t.risk : null));
                              const statusClass = isProtected ? 'status-muted' : (isSkipped || t.risk === 'Caution' || t.risk === 'Unknown' ? (isSkipped ? 'badge status-skipped' : riskClass(t.risk)) : '');
                              return (
                                <div
                                  key={item.id}
                                  className={`scan-row scan-item-row ${checked ? 'selected' : ''} ${inspectedMatch ? 'inspected' : ''}`}
                                  role="button"
                                  tabIndex={0}
                                  onClick={() => { void inspectItem(item.id); if (selectable) toggleItem(item, t); }}
                                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); void inspectItem(item.id); if (selectable) toggleItem(item, t); } }}
                                >
                                  <div className="scan-col-main">
                                    <span className="row-indent" aria-hidden="true" />
                                    <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
                                      {isProtected ? (
                                        <span className="lock-slot" aria-label={`${normalizeDisplayPath(item.path)} protected`} title="Protected">
                                          <LockIcon />
                                        </span>
                                      ) : (
                                        <label className="checkbox">
                                          <input type="checkbox" aria-label={`Select ${normalizeDisplayPath(item.path)}`} checked={checked} disabled={!selectable || cleaning} onChange={() => toggleItem(item, t)} />
                                        </label>
                                      )}
                                    </span>
                                    <div className="scan-main-text">
                                      <div className="scan-title-line">
                                        <span className="item-name" title={normalizeDisplayPath(item.path)}>{name}</span>
                                      </div>
                                      {rootPath && <div className="scan-subpath" title={rootPath}>{rootPath}</div>}
                                    </div>
                                  </div>
                                  <div className="scan-col-status">
                                    {statusLabel ? <span className={statusClass} aria-label={`Status ${isSkipped ? 'Skipped' : statusLabel}`}>{statusLabel}</span> : null}
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
                  {inspected ? (() => {
                    let inspectedItem: ScanItemDto | null = null;
                    let inspectedTarget: TargetScanResultDto | null = null;
                    for (const t of targets) {
                      const it = t.items.find((i) => i.id === inspected.itemId);
                      if (it) { inspectedItem = it; inspectedTarget = t; break; }
                    }
                    const itemTitle = inspectedItem ? relativeItemName(inspectedItem.path, inspectedTarget?.resolvedPaths ?? []) : inspected.title;
                    const itemSubtitle = inspectedTarget?.targetName ?? null;
                    const fullPath = inspectedItem ? normalizeDisplayPath(inspectedItem.path) : normalizeDisplayPath(inspected.itemId);
                    const eligibility = inspectedItem && inspectedTarget
                      ? (isSelectableScanItem(inspectedItem, inspectedTarget, allowDeleteUnknown) ? 'Eligible for cleanup' : humanizeSkippedReason(inspectedTarget, inspectedItem, allowDeleteUnknown))
                      : '—';
                    const whyFlagged = inspectedItem && inspectedTarget
                      ? (inspectedTarget.risk === 'Protected' ? 'Protected Path. This location is never removable.'
                        : inspectedTarget.risk === 'Unknown' && !allowDeleteUnknown ? 'Unknown item and "Allow deleting unknown folders" is off in Settings.'
                          : inspectedItem.ageEligible === false ? 'Modified within the minimum age window and skipped for safety.'
                            : inspected.risk === 'Caution' ? 'Caution category. Review before removal.'
                              : inspected.risk === 'Unknown' ? 'Unknown/Custom Path item. Typed confirmation is required before preview/clean.'
                                : 'Safe category and eligible under current settings.')
                      : inspected.description;
                    return (
                      <>
                        <p className="inspector-headline">
                          <span className="inspector-headline-name" title={fullPath}>{itemTitle}</span>
                          <span className={riskClass(inspected.risk)}>{inspected.risk}</span>
                        </p>
                        {itemSubtitle && <p className="inspector-subtitle muted">{itemSubtitle}</p>}
                        <dl className="kv kv-stacked">
                          <div className="kv-block"><dt>What is it?</dt><dd>{inspected.description}</dd></div>
                          <div className="kv-block"><dt>What happens if deleted?</dt><dd>{inspected.risk === 'Protected' ? 'Nothing will be deleted. Protected Paths are never removable.' : 'The selected item is permanently deleted and bypasses the Recycle Bin. Only items you explicitly select and confirm are removed.'}</dd></div>
                          <div className="kv-block"><dt>Why flagged / skipped?</dt><dd>{whyFlagged}</dd></div>
                          <div className="kv-block"><dt>Eligibility</dt><dd>{eligibility}</dd></div>
                          <div className="kv-block kv-path-block"><dt>Full Path</dt>
                            <dd>
                              <div className="path-well">
                                <span className="mono path-well-text" title={fullPath}>{fullPath}</span>
                                <button type="button" className="icon-only-btn subtle path-copy-btn" onClick={() => void copyText(fullPath)} aria-label="Copy path" title="Copy path"><CopyIcon /></button>
                              </div>
                            </dd>
                          </div>
                        </dl>
                        <div className="row inspector-actions">
                          <button className="btn small" onClick={() => void onExplain(inspected.itemId)} aria-label="Explain">Explain</button>
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

      <ExplainDrawer explain={drawerExplain} onClose={() => setDrawerExplain(null)} />
    </div>
  );
}
