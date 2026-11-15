import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useState } from 'react';
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
import { formatBytes, middleTruncatePath, normalizeDisplayPath, pluralize, sumDecimalStrings } from '../utils/format';
import {
  CleanSummaryBlock,
  formatModifiedAge,
  InfoTip,
  InspectorMetaGrid,
  InspectorTitleRow,
  PathActions,
  PathWell,
  PreviewBreakdownBlock,
  SESSION_TTL_MS,
  TYPED_CONFIRMATION_PHRASE,
  type ExplainState,
  type PreviewResult,
} from './cleanShared';
import {
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  FileIcon,
  FolderIcon,
  RefreshIcon,
  ShieldIcon,
} from './Icons';

type ScanFilter = 'all' | 'eligible' | 'skipped';
type ScanSort = 'size' | 'name';
type ScanToolbarState = {
  query: string;
  sort: ScanSort;
  filter: ScanFilter;
};
type FocusState = { kind: 'item'; itemId: string } | { kind: 'group'; targetId: string } | null;
const VISIBLE_CHILD_CAP = 25;

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
  if (target.risk === 'Unknown' && !allowDeleteUnknown) return 'Skipped · Unverified';
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

function childStatusForScan(target: TargetScanResultDto, item: ScanItemDto, selectable: boolean, allowDeleteUnknown: boolean): { label: string; kind: 'safe' | 'skipped' | 'caution' | 'unknown' | 'protected' } {
  if (target.risk === 'Protected') return { label: 'Protected', kind: 'protected' };
  if (!selectable) {
    if ((isCustomPathTargetId(target.targetId) && target.risk === 'Unknown') || (target.risk === 'Unknown' && !allowDeleteUnknown)) return { label: 'Skipped · Unverified', kind: 'skipped' };
    if (!item.ageEligible) return { label: 'Skipped · < 24h', kind: 'skipped' };
    return { label: humanizeSkippedReason(target, item, allowDeleteUnknown), kind: 'skipped' };
  }
  if (target.risk === 'Caution') return { label: 'Caution', kind: 'caution' };
  if (target.risk === 'Unknown') return { label: 'Unknown', kind: 'unknown' };
  return { label: 'Safe', kind: 'safe' };
}

function statusClassForKind(kind: 'safe' | 'skipped' | 'caution' | 'unknown' | 'protected' | 'eligible', label?: string): string {
  if (kind === 'eligible') return 'status-pill status-pill-safe status-pill-scan';
  if (label === 'Skipped · Unverified') return 'status-pill status-pill-unknown status-pill-scan badge-unknown';
  switch (kind) {
    case 'safe': return 'status-pill status-pill-safe status-pill-scan';
    case 'skipped': return 'status-pill status-pill-caution status-pill-scan';
    case 'caution': return 'status-pill status-pill-caution status-pill-scan';
    case 'unknown': return 'status-pill status-pill-unknown status-pill-scan badge-unknown';
    case 'protected': return 'status-pill status-pill-protected status-pill-scan';
    default: return 'status-pill status-pill-scan';
  }
}

function statusDisplayLabel(kind: 'safe' | 'skipped' | 'caution' | 'unknown' | 'protected' | 'eligible', label: string): string {
  if (kind === 'eligible') return 'Eligible';
  if (kind === 'safe') return 'Safe';
  return label;
}

function scanGuidance(target: TargetScanResultDto, item: ScanItemDto, selectable: boolean, allowDeleteUnknown: boolean): string {
  if (target.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (isCustomPathTargetId(target.targetId) && target.risk === 'Unknown' && !allowDeleteUnknown) return 'Unverified custom folder is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible, then review carefully.';
  if (target.risk === 'Unknown' && !allowDeleteUnknown) return 'Unknown item is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible.';
  if (target.risk === 'Unknown' && isCustomPathTargetId(target.targetId)) return 'Unverified custom folder item. Review carefully; typed DELETE confirmation is required before cleaning.';
  if (target.risk === 'Unknown') return 'Unknown item. Review carefully; typed DELETE confirmation is required before cleaning.';
  if (!item.ageEligible) return 'Modified within the minimum age window. Sweeper skips recent files for safety.';
  if (target.targetName === 'User Temp') return 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.';
  if (target.targetName === 'Crash Dumps & WER') return 'Local crash and error reports kept for diagnostics. Removing them does not affect app functionality.';
  if (target.targetName.toLowerCase().includes('npm')) return 'Package manager download cache. It can be redownloaded automatically when needed.';
  if (isCustomPathTargetId(target.targetId)) return 'Custom folder item. Only selected contents are cleaned and protected system directories remain excluded.';
  if (target.risk === 'Caution') return 'Caution category. Review before removal; selected items are permanently deleted.';
  if (target.risk === 'Safe') return 'Safe to remove when not in active use. Selected items are permanently deleted after review and confirmation.';
  return 'Eligible under current safety settings. Selected items are permanently deleted after review and confirmation.';
}

export type ScanAndPreviewViewHandle = { runScan: () => Promise<void> | void };

export const ScanAndPreviewView = forwardRef<ScanAndPreviewViewHandle, {
  client: SweeperClient;
  settings: Settings | null;
  settingsVersion?: number;
  elevationStatus: ElevationStatus;
  elevationError: string | null;
  onOpenLog: (logId: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}>(function ScanAndPreviewView({
  client,
  settings,
  settingsVersion,
  elevationStatus,
  elevationError,
  onOpenLog,
  onOpenSettings,
}, ref) {
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
  const [focus, setFocus] = useState<FocusState>(null);
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
  const [showAllGroups, setShowAllGroups] = useState<Set<string>>(new Set());
  const [scanSettingsVersion, setScanSettingsVersion] = useState<number | null>(null);
  const [, setTick] = useState(0);

  const targets = useMemo(() => results?.targets ?? [], [results]);
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  useEffect(() => {
    if (!scanStartedAt || !sessionId) return;
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, [scanStartedAt, sessionId]);

  const isSessionExpired = !!scanStartedAt && Date.now() - scanStartedAt > SESSION_TTL_MS;

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

  const allEligibleSelected = useMemo(() => {
    if (eligibleSafeIds.length === 0) return false;
    return eligibleSafeIds.every((id) => selectedIds.has(id));
  }, [eligibleSafeIds, selectedIds]);

  function toggleSelectAllEligibleSafe() {
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (allEligibleSelected) {
        for (const id of eligibleSafeIds) next.delete(id);
      } else {
        for (const id of eligibleSafeIds) next.add(id);
      }
      return next;
    });
  }

  const selectedCategoryCount = useMemo(() => {
    let n = 0;
    for (const t of visibleBaseTargets) {
      if (t.items.some((i) => selectedIds.has(i.id))) n++;
    }
    return n;
  }, [visibleBaseTargets, selectedIds]);

  const selectedCount = selectedIds.size;
  const masterAllSelected = allEligibleSelected;
  const masterIndeterminate = !masterAllSelected && selectedCount > 0;

  const heroReclaimable = useMemo(() => {
    let total = 0n;
    for (const t of visibleBaseTargets) total += BigInt(t.eligibleBytes || '0');
    return total.toString();
  }, [visibleBaseTargets]);

  const settingsStale = !!(results && scanSettingsVersion !== null && settingsVersion !== undefined && scanSettingsVersion !== settingsVersion);

  function resetForInvalidSession(message?: string) {
    setSessionId(null);
    setScanStartedAt(null);
    setResults(null);
    setSelectedIds(new Set());
    setCollapsedTargets(new Set());
    setShowAllGroups(new Set());
    setScanSettingsVersion(null);
    setPreview(null);
    setConfirmText('');
    setCleanProgress(null);
    setFocus(null);
    setInspected(null);
    if (message) setInfo(message);
  }

  async function inspectItem(itemId: string) {
    try { setInspected(await client.explainItem(itemId)); } catch { /* non-fatal */ }
  }

  const runScan = useCallback(async () => {
    setError(null); setInfo(null); setPreview(null); setFocus(null); setInspected(null); setSelectedIds(new Set());
    setCollapsedTargets(new Set()); setShowAllGroups(new Set()); setResults(null); setCleanSummary(null); setCleanProgress(null); setScanning(true);
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
      setScanSettingsVersion(settingsVersion ?? 0);
      const auto = new Set<string>();
      for (const t of data.targets) {
        if (t.risk !== 'Safe') continue;
        for (const i of t.items) if (isSelectableScanItem(i, t, settings?.allowDeleteUnknown ?? false)) auto.add(i.id);
      }
      setSelectedIds(auto);

      const baseVisible = data.targets.filter((t) => !(t.risk === 'Protected' && BigInt(t.totalBytes || '0') === 0n));
      const collapsed = new Set<string>();
      const primary = baseVisible.find((t) => t.targetName === 'User Temp') ?? baseVisible[0] ?? null;
      for (const t of baseVisible) {
        if (primary && t.targetId === primary.targetId) continue;
        collapsed.add(t.targetId);
      }
      setCollapsedTargets(collapsed);

      const firstTarget = primary ?? baseVisible[0];
      const firstItem = firstTarget?.items[0] ?? data.targets.flatMap((t) => t.items)[0];
      if (firstItem) {
        setFocus({ kind: 'item', itemId: firstItem.id });
        try { setInspected(await client.explainItem(firstItem.id)); } catch { /* ignore */ }
      } else {
        setFocus(null);
      }
      setScanStatus('');
    } catch (e) {
      setError(formatContractError(e));
      const ce = e as ContractError | undefined;
      if (ce?.code === 'sessionInvalid') resetForInvalidSession();
    } finally {
      setScanning(false);
    }
  }, [tier1, tier2, client, settings, settingsVersion]);

  useImperativeHandle(ref, () => ({ runScan }), [runScan]);

  useEffect(() => {
    if (!results || scanning || cleaning) return;
    void runScan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tier1, tier2]);

  function focusItem(itemId: string) {
    setFocus({ kind: 'item', itemId });
    void inspectItem(itemId);
  }

  function focusGroup(targetId: string) {
    setFocus({ kind: 'group', targetId });
  }

  function toggleItem(item: ScanItemDto, target: TargetScanResultDto) {
    if (!isSelectableScanItem(item, target, allowDeleteUnknown)) { focusItem(item.id); return; }
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => { const next = new Set(prev); if (next.has(item.id)) next.delete(item.id); else next.add(item.id); return next; });
    focusItem(item.id);
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
      const ids = isFilterActive ? visibleEligibleSafeIds : eligibleSafeIds;
      for (const id of ids) next.add(id);
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

  function toggleShowAll(targetId: string) {
    setShowAllGroups((prev) => { const next = new Set(prev); if (next.has(targetId)) next.delete(targetId); else next.add(targetId); return next; });
  }

  const anyCollapsed = visibleBaseTargets.length > 0 && collapsedTargets.size > 0;
  function toggleExpandCollapseAll() {
    if (anyCollapsed) setCollapsedTargets(new Set());
    else setCollapsedTargets(new Set(visibleBaseTargets.map((t) => t.targetId)));
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading && !cleaning && !isSessionExpired && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !scanning && !previewLoading && !cleaning;

  function customDisplayName(target: TargetScanResultDto): { title: string; tag: string | null } {
    if (target.targetName === 'Custom Path (Unknown)') {
      const base = (resolvedRootPath(target).split('\\').filter(Boolean).pop()) || target.targetName;
      return { title: base, tag: 'Custom Path' };
    }
    return { title: target.targetName, tag: null };
  }

  const q = toolbar.query.trim().toLowerCase();
  const isFilterActive = toolbar.filter !== 'all' || q.length > 0;
  const visibleTargets = useMemo(() => {
    return visibleBaseTargets.map((t) => {
      let items = t.items.filter((i) => {
        const selectable = isSelectableScanItem(i, t, allowDeleteUnknown);
        if (toolbar.filter === 'eligible' && !selectable) return false;
        if (toolbar.filter === 'skipped' && selectable) return false;
        if (q) {
          const rel = relativeItemName(i.path, t.resolvedPaths).toLowerCase();
          const name = rel.split(/[\\/]/).pop()?.toLowerCase() ?? '';
          if (!i.path.toLowerCase().includes(q) && !rel.includes(q) && !name.includes(q)) return false;
        }
        return true;
      });
      items = [...items].sort((a, b) => {
        if (toolbar.sort === 'name') return relativeItemName(a.path, t.resolvedPaths).localeCompare(relativeItemName(b.path, t.resolvedPaths));
        const da = BigInt(a.size), db = BigInt(b.size);
        return da === db ? 0 : (da > db ? -1 : 1);
      });
      return { target: t, items };
    }).filter((g) => g.items.length > 0 || (!q && toolbar.filter === 'all'));
  }, [visibleBaseTargets, toolbar, allowDeleteUnknown, q]);

  const visibleEligibleSafeIds = useMemo(() => {
    const ids: string[] = [];
    for (const { target: t, items } of visibleTargets) {
      if (t.risk !== 'Safe') continue;
      for (const i of items) if (isSelectableScanItem(i, t, allowDeleteUnknown)) ids.push(i.id);
    }
    return ids;
  }, [visibleTargets, allowDeleteUnknown]);

  const selectSafeIds = isFilterActive ? visibleEligibleSafeIds : eligibleSafeIds;
  const allSelectSafeSelected = selectSafeIds.length > 0 && selectSafeIds.every((id) => selectedIds.has(id));
  const selectSafeLabel = isFilterActive ? `Select Visible Safe (${selectSafeIds.length})` : 'Select Safe';
  const selectSafeAriaLabel = isFilterActive ? `Select Visible Safe (${selectSafeIds.length})` : 'Select Safe';

  const hasResults = !!results;

  const inspectedCtx = useMemo(() => {
    if (focus?.kind !== 'item' || !inspected) return null;
    if (inspected.itemId !== focus.itemId) return null;
    for (const t of visibleBaseTargets) {
      const it = t.items.find((i) => i.id === inspected.itemId);
      if (it) return { target: t, item: it };
    }
    return null;
  }, [inspected, visibleBaseTargets, focus]);

  const focusedGroup = useMemo(() => {
    if (focus?.kind !== 'group') return null;
    return visibleBaseTargets.find((t) => t.targetId === focus.targetId) ?? null;
  }, [focus, visibleBaseTargets]);

  const inspectedInCustomUnknownLocked = !!(
    inspectedCtx &&
    isCustomPathTargetId(inspectedCtx.target.targetId) &&
    inspectedCtx.target.risk === 'Unknown' &&
    !allowDeleteUnknown
  );

  const cleanProgressLabel = cleanProgress
    ? `Progress: ${cleanProgress.completedItems} / ${cleanProgress.totalItems} items — ${formatBytes(cleanProgress.bytesFreedSoFar)} freed so far (${cleanProgress.status})`
    : null;

  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header scan-header-row" aria-label="Scan and Clean header">
        <div className="page-header-left scan-header-title-block">
          <div className="page-title-row">
            <h1 className="page-title" id="scan-heading">Scan &amp; Clean</h1>
            {results && <span className="header-subtitle num" aria-label="Reclaimable total">{formatBytes(heroReclaimable)} reclaimable</span>}
          </div>
        </div>
        <div className="page-header-right scan-header-actions scan-header-actions-v19">
          <div className="scan-header-scopes scan-header-scopes-inline scan-header-scopes-subtle" aria-label="Scan scopes" title="Scope changes apply on Rescan">
            <span className="scan-scope-label muted">Scan Scope:</span>
            <div className="scan-scope-inline-controls scan-scope-inline-controls-subtle" role="group" aria-label="Scan scopes">
              <span className="scan-scope-inline-item">
                <span className={`scan-scope-chip scan-scope-pill scan-scope-config-chip${tier1 ? ' active' : ''}`}>
                  <label className="scan-scope-pill-label">
                    <input className="sr-only" type="checkbox" checked={tier1} onChange={(e) => setTier1(e.target.checked)} disabled={scanning || cleaning} aria-label="Temp Files" />
                    <span className="scan-scope-pill-text">{tier1 ? '✓ ' : ''}Temp Files</span>
                  </label>
                  <span className="scan-scope-pill-info">
                    <InfoTip label="About System &amp; App Temp (Tier 1)">
                      <p><strong>System &amp; App Temp (Tier 1)</strong><br />Windows Temp, crash dumps &amp; app caches older than 24h</p>
                    </InfoTip>
                  </span>
                </span>
              </span>
              <span className="scan-scope-inline-item">
                <span className={`scan-scope-chip scan-scope-pill scan-scope-config-chip${tier2 ? ' active' : ''}`}>
                  <label className="scan-scope-pill-label">
                    <input className="sr-only" type="checkbox" checked={tier2} onChange={(e) => setTier2(e.target.checked)} disabled={scanning || cleaning} aria-label="Dev Caches" />
                    <span className="scan-scope-pill-text">{tier2 ? '✓ ' : ''}Dev Caches</span>
                  </label>
                  <span className="scan-scope-pill-info">
                    <InfoTip label="About Developer Caches (Tier 2)">
                      <p><strong>Developer Caches (Tier 2)</strong><br />npm, Cargo, pip &amp; build caches that rebuild automatically. Scope changes apply on the next Rescan. Matches <strong>Settings → General</strong>.</p>
                    </InfoTip>
                  </span>
                </span>
              </span>
              {onOpenSettings ? (
                <button
                  type="button"
                  className={`scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip${settings?.enableTier3 ? ' active' : ''}`}
                  onClick={() => onOpenSettings('enableTier3')}
                  aria-label={settings?.enableTier3 ? 'Deep Cleanup: On · Settings' : 'Deep Cleanup: Off · Configure'}
                  title={settings?.enableTier3 ? 'Deep Cleanup is on (configure in Settings)' : 'Configure Deep Cleanup in Settings'}
                >
                  <span className="scan-scope-pill-label scan-scope-pill-label-static">
                    <span className="scan-scope-pill-text">{settings?.enableTier3 ? 'Deep Cleanup: On' : 'Deep Cleanup: Off'} <span aria-hidden="true">⚙</span></span>
                  </span>
                </button>
              ) : (
                <span className={`scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip ${settings?.enableTier3 ? 'active on' : 'off'}`} aria-label={`Deep Cleanup: ${settings?.enableTier3 ? 'On' : 'Off'}`}>
                  <span className="scan-scope-pill-label scan-scope-pill-label-static">
                    <span className="scan-scope-pill-text">Deep Cleanup: {settings?.enableTier3 ? 'On' : 'Off'} <span aria-hidden="true">⚙</span></span>
                  </span>
                </span>
              )}
            </div>
          </div>
          <div className="scan-header-rescan-wrap">
            {settingsStale && <span className="settings-updated-pill" role="status">Settings updated — Rescan to apply</span>}
            <button
              key={hasResults ? 'rescan' : 'scan'}
              className={hasResults ? 'btn secondary-action scan-rescan-btn' : 'btn primary scan-rescan-btn'}
              onClick={runScan}
              disabled={scanning || cleaning}
              aria-label="Start Scan"
            >
              {hasResults ? <><RefreshIcon aria-hidden="true" /> <span>Rescan</span></> : 'Start Scan'}
            </button>
          </div>
        </div>
      </header>

      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {relaunchInfo && <div className="notice info" role="status">{relaunchInfo}</div>}
      {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}
      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      <section className="scan-controls scan-controls-compact" aria-label="Scan controls">
        <div className="scan-table-toolbar scan-table-toolbar-mainbar" aria-label="Scan table toolbar">
          <div className="scan-table-toolbar-left">
            <div className="segmented-toggle scan-status-segmented" role="group" aria-label="Status filters">
              <button type="button" className={`seg-btn ${toolbar.filter === 'all' ? 'active' : ''}`} aria-pressed={toolbar.filter === 'all'} onClick={() => setToolbar({ ...toolbar, filter: 'all' })}>All ({visibleBaseTargets.length} categories · {totalItemCount} files)</button>
              <button type="button" className={`seg-btn ${toolbar.filter === 'eligible' ? 'active' : ''}`} aria-pressed={toolbar.filter === 'eligible'} aria-label={`Eligible (${selectableCount} files)`} onClick={() => setToolbar({ ...toolbar, filter: 'eligible' })}>Safe ({selectableCount})</button>
              <button type="button" className={`seg-btn ${toolbar.filter === 'skipped' ? 'active' : ''}`} aria-pressed={toolbar.filter === 'skipped'} onClick={() => setToolbar({ ...toolbar, filter: 'skipped' })}>Skipped ({skippedCount})</button>
            </div>
          </div>
          <div className="scan-table-toolbar-middle">
            <input
              type="search"
              className="scan-files-search"
              placeholder="Search files..."
              value={toolbar.query}
              onChange={(e) => setToolbar({ ...toolbar, query: e.target.value })}
              aria-label="Search files"
              disabled={!results}
            />
          </div>
          <div className="scan-table-toolbar-right">
            <label className="toolbar-group scan-sort-group">Sort
              <select value={toolbar.sort} onChange={(e) => setToolbar({ ...toolbar, sort: e.target.value as ScanSort })} aria-label="Sort scan results" disabled={!results}>
                <option value="size">Size</option>
                <option value="name">Name</option>
              </select>
            </label>
            <button className="btn small subtle-btn" onClick={toggleExpandCollapseAll} disabled={cleaning || !results}>{anyCollapsed ? 'Expand All' : 'Collapse All'}</button>
            <button className="btn small" onClick={selectAllSafe} disabled={cleaning || selectSafeIds.length === 0} aria-label={selectSafeAriaLabel} title={allSelectSafeSelected ? 'All safe items selected' : undefined}>{allSelectSafeSelected ? `✓ ${selectSafeLabel}` : selectSafeLabel}</button>
          </div>
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
            <div className="scan-table-header">
              <span className="scan-th-item">
                <label className="checkbox scan-master-check" aria-label={masterAllSelected ? 'Deselect all eligible files' : 'Select all eligible files'}>
                  <input
                    type="checkbox"
                    checked={masterAllSelected}
                    aria-checked={masterIndeterminate ? 'mixed' : masterAllSelected}
                    ref={(el) => { if (el) el.indeterminate = masterIndeterminate; }}
                    disabled={eligibleSafeIds.length === 0 || scanning || cleaning}
                    onChange={toggleSelectAllEligibleSafe}
                    aria-label={masterAllSelected ? 'Deselect all' : 'Select all eligible'}
                  />
                </label>
                <span>Item</span>
              </span>
              <span className="scan-th-items num">Items</span>
                <span className="scan-th-size num">Size</span>
                <span className="scan-th-status">Status</span>
            </div>
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
              <div className="scan-table-header">
                <span className="scan-th-item">
                  <label className="checkbox scan-master-check" aria-label={masterAllSelected ? 'Deselect all eligible files' : 'Select all eligible files'}>
                    <input
                      type="checkbox"
                      checked={masterAllSelected}
                      aria-checked={masterIndeterminate ? 'mixed' : masterAllSelected}
                      ref={(el) => { if (el) el.indeterminate = masterIndeterminate; }}
                      disabled={eligibleSafeIds.length === 0 || scanning || cleaning}
                      onChange={toggleSelectAllEligibleSafe}
                      aria-label={masterAllSelected ? 'Deselect all' : 'Select all eligible'}
                    />
                  </label>
                  <span>Item</span>
                </span>
                <span className="scan-th-items num">Items</span>
                <span className="scan-th-size num">Size</span>
                <span className="scan-th-status">Status</span>
              </div>
              <div className="pane-body flush scan-results-body scan-results-shell">
                {visibleBaseTargets.length === 0 && <div className="empty-state"><h3>No targets</h3><p>No Targets returned for this Scan.</p></div>}
                <div className="target-list scan-grid-list scan-table-scroll">
                  {visibleTargets.map(({ target: t, items: visibleItems }) => {
                    const selectableItems = t.items.filter((i) => isSelectableScanItem(i, t, allowDeleteUnknown));
                    const selectableIds = selectableItems.map((i) => i.id);
                    const selectedInTarget = selectableIds.filter((id) => selectedIds.has(id)).length;
                    const allEligibleInGroupSelected = selectableIds.length > 0 && selectedInTarget === selectableIds.length;
                    const isFullySelected = t.items.length > 0 && selectedInTarget === t.items.length;
                    const isIndeterminate = selectedInTarget > 0 && selectedInTarget < t.items.length;
                    const collapsed = collapsedTargets.has(t.targetId);
                    const firstItem = t.items[0];
                    const visibleAll = showAllGroups.has(t.targetId);
                    const limitedItems = !visibleAll && visibleItems.length > VISIBLE_CHILD_CAP ? visibleItems.slice(0, VISIBLE_CHILD_CAP) : visibleItems;
                    const hiddenCount = !visibleAll ? Math.max(0, visibleItems.length - limitedItems.length) : 0;
                    const isCustomUnknownLocked = isCustomPathTargetId(t.targetId) && t.targetName === 'Custom Path (Unknown)' && t.risk === 'Unknown' && !allowDeleteUnknown;
                    const display = customDisplayName(t);
                    const rootPath = resolvedRootPath(t);
                    const rootPathDisplay = middleTruncatePath(rootPath, 64);
                    const totalScannedBytes = BigInt(t.totalBytes || '0');
                    const eligibleBytesBig = isCustomUnknownLocked ? 0n : selectableItems.reduce((acc, it) => acc + BigInt(it.size || '0'), 0n);
                    const showOfTotal = totalScannedBytes > eligibleBytesBig;
                    const expandLabel = collapsed ? `Expand ${t.targetName}` : `Collapse ${t.targetName}`;
                    const totalInTarget = t.items.length;
                    const itemsLabel = `${selectedInTarget} of ${totalInTarget} selected`;
                    const groupActive = focus?.kind === 'group' && focus.targetId === t.targetId;
                    const eligibleInTarget = selectableIds.length;
                    const skippedInTargetCount = totalInTarget - eligibleInTarget;
                    let groupStatus: { label: string; kind: 'safe' | 'skipped' | 'caution' | 'unknown' | 'protected' | 'eligible'; aria: string } | null = null;
                    if (isCustomUnknownLocked) {
                      groupStatus = { label: 'Skipped · Unverified', kind: 'skipped', aria: 'Status Skipped' };
                    } else if (t.risk === 'Protected') {
                      groupStatus = { label: 'Protected', kind: 'protected', aria: 'Status Protected' };
                    } else if (eligibleInTarget === 0 && totalInTarget > 0) {
                      groupStatus = { label: 'Skipped', kind: 'skipped', aria: 'Status Skipped' };
                    } else if (eligibleInTarget > 0 && skippedInTargetCount === 0) {
                      groupStatus = { label: 'Safe', kind: 'safe', aria: 'Status Safe' };
                    } else if (eligibleInTarget > 0 && skippedInTargetCount > 0) {
                      groupStatus = eligibleInTarget === 1
                        ? { label: '1 Safe', kind: 'safe', aria: 'Status Safe' }
                        : { label: 'Partial', kind: 'safe', aria: 'Status Safe' };
                    } else {
                      groupStatus = null;
                    }

                    return (
                      <article key={t.targetId} className={`target-card ${collapsed ? 'collapsed' : 'expanded'}`} aria-labelledby={`target-${t.targetId}-name`}>
                        <div
                          className={`scan-row scan-group-row${groupActive ? ' active-group' : ''}`}
                          role="button"
                          tabIndex={0}
                          aria-label={`Inspect ${t.targetName}`}
                          onClick={() => focusGroup(t.targetId)}
                          onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); focusGroup(t.targetId); } }}
                        >
                          <div className="scan-col-main">
                            <button type="button" className="target-collapse-btn" onClick={(e) => { e.stopPropagation(); toggleCollapsed(t.targetId); }} aria-label={expandLabel}>
                              {collapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}
                            </button>
                            <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
                              {t.risk === 'Protected' ? (
                                <>
                                  <span className="fixed-check-placeholder" aria-hidden="true">—</span>
                                  <input className="sr-only" type="checkbox" aria-label={`Select all eligible items in ${t.targetName}`} disabled checked={false} readOnly tabIndex={-1} />
                                </>
                              ) : (
                                <label className="checkbox target-group-check">
                                  <input
                                    type="checkbox"
                                    aria-label={`Select all eligible items in ${t.targetName}`}
                                    checked={isFullySelected}
                                    aria-checked={isIndeterminate ? 'mixed' : isFullySelected}
                                    ref={(el) => { if (el) el.indeterminate = isIndeterminate; }}
                                    disabled={cleaning || selectableIds.length === 0}
                                    onChange={() => setTargetSelection(t, !allEligibleInGroupSelected)}
                                  />
                                </label>
                              )}
                            </span>
                            <FolderIcon className="row-icon" aria-hidden="true" />
                            <div className="scan-main-text scan-group-main-text">
                              <div className="scan-title-line scan-title-line-with-action">
                                <span className="target-name-btn scan-group-title" id={`target-${t.targetId}-name`}>{display.title}</span>
                                {display.tag && <span className="scan-custom-path-tag muted">{display.tag}</span>}
                                {t.targetName === 'Custom Path (Unknown)' && <span className="sr-only">Custom Path (Unknown)</span>}
                              </div>
                              <div className="scan-subpath scan-group-subpath" title={rootPath}>
                                <span className="mono scan-group-path-text">{rootPathDisplay || '\u00A0'}</span>
                              </div>
                            </div>
                          </div>
                          <div className="scan-col-items muted num">{itemsLabel}</div>
                          <div className="scan-col-size scan-group-size-col num">
                            <div className="scan-group-size-primary">{formatBytes(eligibleBytesBig.toString())}</div>
                            {showOfTotal && <div className="scan-group-size-secondary muted">{`of ${formatBytes(t.totalBytes)}`}</div>}
                          </div>
                          <div className="scan-col-status scan-group-status-col">
                            {groupStatus ? (
                              isCustomUnknownLocked && onOpenSettings ? (
                                <button
                                  type="button"
                                  className={`${statusClassForKind(groupStatus.kind, groupStatus.label)} scan-status-action-pill scan-status-action-pill-interactive`}
                                  aria-label={groupStatus.aria}
                                  title="Click to configure in Settings"
                                  onClick={(e) => { e.stopPropagation(); onOpenSettings('allowDeleteUnknown', resolvedRootPath(t)); }}
                                >
                                  {statusDisplayLabel(groupStatus.kind, groupStatus.label)} →
                                </button>
                              ) : (
                                <span className={statusClassForKind(groupStatus.kind, groupStatus.label)} aria-label={groupStatus.aria}>{statusDisplayLabel(groupStatus.kind, groupStatus.label)}</span>
                              )
                            ) : <span className="scan-status-empty muted">—</span>}
                          </div>
                        </div>
                        {!collapsed && (
                          <div className="items" role="group" aria-label={`${t.targetName} items`}>
                            <div className="scan-items-guide" aria-hidden="true" />
                            {t.items.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items.</span></div>}
                            {t.items.length > 0 && visibleItems.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items shown for current filters.</span></div>}
                            {limitedItems.map((item) => {
                              const selectable = isSelectableScanItem(item, t, allowDeleteUnknown);
                              const checked = selectedIds.has(item.id);
                              const { name, nestedRel } = itemDisplayName(item.path, t.resolvedPaths);
                              const isProtected = t.risk === 'Protected';
                              const inspectedMatch = focus?.kind === 'item' && focus.itemId === item.id;
                              const status = childStatusForScan(t, item, selectable, allowDeleteUnknown);
                              const isStatusSkipped = status.label.startsWith('Skipped') || status.kind === 'skipped';
                              return (
                                <div
                                  key={item.id}
                                  className={`scan-row scan-item-row scan-row-child scan-child-row ${inspectedMatch ? 'inspected active' : ''} ${checked ? 'selected' : ''}`}
                                  role="button"
                                  tabIndex={0}
                                  onClick={() => toggleItem(item, t)}
                                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleItem(item, t); } }}
                                >
                                  <div className="scan-col-main">
                                    <span className="row-indent" aria-hidden="true" />
                                    <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
                                      {isProtected ? (
                                        <>
                                          <span className="fixed-check-placeholder" aria-hidden="true">—</span>
                                          <input className="sr-only" type="checkbox" aria-label={`Select ${normalizeDisplayPath(item.path)}`} disabled checked={checked} readOnly tabIndex={-1} />
                                        </>
                                      ) : !selectable ? (
                                        <>
                                          <span className="scan-disabled-checkbox-outline" aria-hidden="true" />
                                          <input className="sr-only" type="checkbox" aria-label={`Select ${normalizeDisplayPath(item.path)}`} disabled checked={checked} readOnly tabIndex={-1} />
                                        </>
                                      ) : (
                                        <label className="checkbox">
                                          <input type="checkbox" aria-label={`Select ${normalizeDisplayPath(item.path)}`} checked={checked} disabled={cleaning} onChange={() => toggleItem(item, t)} />
                                        </label>
                                      )}
                                    </span>
                                    <FileIcon className="row-icon" aria-hidden="true" />
                                    <div className="scan-main-text">
                                      <div className="scan-title-line">
                                        <span className="item-name" title={normalizeDisplayPath(item.path)}>{middleTruncatePath(name, 64)}</span>
                                        {nestedRel && nestedRel !== name ? <span className="scan-subpath-inline mono" title={nestedRel}>{middleTruncatePath(nestedRel, 64)}</span> : null}
                                      </div>
                                    </div>
                                  </div>
                                  <div className="scan-col-items muted" />
                                  <div className="scan-col-size num">{formatBytes(item.size)}</div>
                                  <div className="scan-col-status">
                                    <span className={statusClassForKind(status.kind, status.label)} aria-label={isStatusSkipped ? 'Status Skipped' : `Status ${status.label}`}>
                                      {statusDisplayLabel(status.kind, status.label)}
                                    </span>
                                  </div>
                                </div>
                              );
                            })}
                            {hiddenCount > 0 && (
                              <div className="scan-row scan-item-empty scan-show-all-row">
                                <button type="button" className="btn small subtle-btn" onClick={() => toggleShowAll(t.targetId)}>{`Showing top ${VISIBLE_CHILD_CAP} of ${visibleItems.length} files · Show more`}</button>
                              </div>
                            )}
                            {visibleAll && visibleItems.length > VISIBLE_CHILD_CAP && (
                              <div className="scan-row scan-item-empty scan-show-all-row">
                                <button type="button" className="btn small subtle-btn" onClick={() => toggleShowAll(t.targetId)}>Show fewer files</button>
                              </div>
                            )}
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
                  {focus?.kind === 'group' && focusedGroup ? (() => {
                    const t = focusedGroup;
                    const selectableItems = t.items.filter((i) => isSelectableScanItem(i, t, allowDeleteUnknown));
                    const selectedInTarget = selectableItems.filter((i) => selectedIds.has(i.id)).length;
                    const skippedInTarget = t.items.length - selectableItems.length;
                    const isCustomUnknownLocked = isCustomPathTargetId(t.targetId) && t.targetName === 'Custom Path (Unknown)' && t.risk === 'Unknown' && !allowDeleteUnknown;
                    const groupStatusLabel = isCustomUnknownLocked ? 'Skipped · Unverified' : (selectedInTarget > 0 ? 'Eligible' : t.risk);
                    const groupStatusKind = isCustomUnknownLocked ? 'skipped' : (selectedInTarget > 0 ? 'eligible' : (t.risk === 'Protected' ? 'protected' : t.risk === 'Caution' ? 'caution' : t.risk === 'Unknown' ? 'unknown' : 'safe'));
                    const display = customDisplayName(t);
                    const topFiles = [...t.items].sort((a, b) => BigInt(b.size) === BigInt(a.size) ? 0 : (BigInt(b.size) > BigInt(a.size) ? 1 : -1)).slice(0, 5);
                    const eligibleBytes = sumDecimalStrings(selectableItems.map((i) => i.size));
                    const skippedBytes = sumDecimalStrings(t.items.filter((i) => !isSelectableScanItem(i, t, allowDeleteUnknown)).map((i) => i.size));
                    const policyText = t.targetName === 'User Temp'
                      ? 'Policy: Minimum age > 24h · Auto-recreated by Windows/apps'
                      : `Policy: Minimum age > ${settings?.minAgeHours ?? 24}h · Governed by ${t.targetName} safety rules`;
                    const desc = t.items[0] ? (t.targetName === 'User Temp' ? 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.' : (t.statusReason || 'Category-level cleanup target governed by Sweeper safety rules.')) : (t.statusReason || 'Category-level cleanup target governed by Sweeper safety rules.');
                    return (
                      <>
                        <InspectorTitleRow
                          icon={<FolderIcon className="inspector-leading-icon" />}
                          title={display.title}
                          subtitle="Category Overview"
                          badge={<span className={statusClassForKind(groupStatusKind, groupStatusLabel)}>{groupStatusLabel}</span>}
                        />
                        <p className="inspector-guidance">{desc}</p>
                        <dl className="inspector-meta-grid inspector-meta-grid-v19" aria-label="Category metadata">
                          <div><dt>Reclaimable</dt><dd className="num">{formatBytes(eligibleBytes || '0')}</dd></div>
                          <div><dt>Skipped</dt><dd className="num">{formatBytes(skippedBytes || '0')}</dd></div>
                          <div className="inspector-meta-eligibility inspector-meta-eligibility-full"><dt>Items</dt><dd>{selectedInTarget} selected · {skippedInTarget} skipped · {pluralize(t.items.length, 'file', 'files')} total</dd></div>
                        </dl>
                        <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety">
                          <div className="inspector-policy-title">Rule &amp; Safety</div>
                          <div className="inspector-policy-text"><strong>{policyText}</strong></div>
                        </div>
                        <div className="inspector-top-files" aria-label="Top files preview">
                          <strong className="subheading-xs">Top files</strong>
                          {topFiles.length === 0 ? <p className="muted small-note">No files in this category.</p> : (
                            <ul className="inspector-top-files-list">
                              {topFiles.map((item) => {
                                const name = itemDisplayName(item.path, t.resolvedPaths).name;
                                const st = childStatusForScan(t, item, isSelectableScanItem(item, t, allowDeleteUnknown), allowDeleteUnknown);
                                return <li key={item.id}><span className="inspector-top-file-name" title={normalizeDisplayPath(item.path)}>{name}</span><span className="num muted">{formatBytes(item.size)}</span><span className="muted">{statusDisplayLabel(st.kind, st.label)}</span></li>;
                              })}
                            </ul>
                          )}
                        </div>
                        {isCustomUnknownLocked && onOpenSettings && (
                          <div className="inspector-policy-links inspector-allow-settings-prominent" aria-label="Inspector actions">
                            <div className="inspector-rule-safety-block">
                              <div className="inspector-rule-safety-title">ELIGIBILITY / RULE &amp; SAFETY</div>
                              <p className="muted small-note">Unverified custom folder is locked by Settings. Enable custom path cleanup to make it eligible.</p>
                            </div>
                            <button type="button" className="btn small secondary-action inspector-inline-link prominent-allow-link" onClick={() => onOpenSettings('allowDeleteUnknown', resolvedRootPath(t))}>Allow in Settings →</button>
                          </div>
                        )}
                      </>
                    );
                  })() : inspected && inspectedCtx && focus?.kind === 'item' ? (() => {
                    const { item, target } = inspectedCtx;
                    const selectable = isSelectableScanItem(item, target, allowDeleteUnknown);
                    const fullPath = normalizeDisplayPath(item.path);
                    const itemTitle = relativeItemName(item.path, target.resolvedPaths).split(/[\\/]/).pop() || inspected.title;
                    const status = childStatusForScan(target, item, selectable, allowDeleteUnknown);
                    const eligibility = selectable ? 'Meets age limit (> 24h old)' : humanizeSkippedReason(target, item, allowDeleteUnknown);
                    const modifiedAge = formatModifiedAge(item.mtimeUnixSecs, item.ageEligible);
                    const isVhdx = fullPath.toLowerCase().endsWith('.vhdx');
                    const isCustomUnknownLocked = isCustomPathTargetId(target.targetId) && target.risk === 'Unknown' && !allowDeleteUnknown;
                    const isSkippedByAge = !selectable && !item.ageEligible;
                    const display = customDisplayName(target);
                    return (
                      <>
                        <InspectorTitleRow
                          icon={<FileIcon className="inspector-leading-icon" />}
                          title={itemTitle}
                          subtitle={display.tag ? display.title : target.targetName}
                          badge={<span className={statusClassForKind(status.kind, status.label)} aria-label={status.label.startsWith('Skipped') || status.kind === 'skipped' ? 'Status Skipped' : `Status ${status.label}`}>{statusDisplayLabel(status.kind, status.label)}</span>}
                        />
                        <p className="inspector-guidance">{scanGuidance(target, item, selectable, allowDeleteUnknown)}</p>
                        <InspectorMetaGrid
                          size={item.size}
                          modifiedAge={modifiedAge}
                          eligibility={eligibility}
                        />
                        <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety">
                          <div className="inspector-policy-title">Rule &amp; Safety</div>
                          <div className="inspector-policy-text"><strong>Policy:</strong> {target.targetName === 'User Temp' ? 'Minimum age > 24h · Auto-recreated by Windows/apps' : `Minimum age > ${settings?.minAgeHours ?? 24}h · Governed by ${target.targetName} safety rules`}</div>
                        </div>
                        {isVhdx && (
                          <div className="notice info" role="note">
                            <div><strong>.vhdx is display-only.</strong> To compact: shut down WSL/VM (<code>wsl --shutdown</code>) then run <code>Optimize-VHD</code> (Hyper-V) or your VM tooling.</div>
                          </div>
                        )}
                        <PathWell path={fullPath} />
                        <PathActions path={fullPath} />
                        {(isCustomUnknownLocked || isSkippedByAge) && onOpenSettings && (
                          <div className="inspector-policy-links" aria-label="Inspector actions">
                            {isCustomUnknownLocked && (
                              <button type="button" className="btn small inspector-inline-link" onClick={() => onOpenSettings('allowDeleteUnknown', resolvedRootPath(target))}>Allow in Settings →</button>
                            )}
                            {isSkippedByAge && (
                              <button type="button" className="btn small inspector-inline-link" onClick={() => onOpenSettings()}>Adjust minimum age in Settings →</button>
                            )}
                          </div>
                        )}
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

                {preview && <PreviewBreakdownBlock preview={preview} cleaning={cleaning} cleanProgressLabel={cleanProgressLabel} />}
                {cleanSummary && <CleanSummaryBlock summary={cleanSummary} onOpenLog={onOpenLog} />}
              </div>
            </aside>
          </section>

          <div className="scan-bottom-bar" aria-label="Cleanup actions">
            <div className="scan-bottom-bar-left">
              <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(selectedIds.size, 'file', 'files')} selected</strong> ({pluralize(selectedCategoryCount, 'category', 'categories')}) · <span className="num">{formatBytes(selectedBytes)}</span> reclaimable</span>
              {selectedIds.size > 0 && <button type="button" className="btn small subtle-btn" onClick={clearSelection} disabled={cleaning}>Clear</button>}
              <span className="scan-bottom-bar-hint muted">Next: Review summary · permanent delete (skips Recycle Bin)</span>
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
});
