import { forwardRef, useImperativeHandle, useMemo } from 'react';
import { formatContractError, type ElevationStatus, type Settings, type SweeperClient } from '../api/client';
import type { ContractError, ScanItemDto, TargetScanResultDto } from '../contract/bindings';
import { formatBytes, sumDecimalStrings } from '../utils/format';
import { InfoTip, TYPED_CONFIRMATION_PHRASE } from './cleanShared';
import { RefreshIcon } from './Icons';
import { ScanCleanBar } from './scan/ScanCleanBar';
import { ScanInspectorPanel } from './scan/ScanInspectorPanel';
import { ScanTargetCard } from './scan/ScanTargetCard';
import { isSelectableScanItem, scanSelectionNeedsConfirmation } from './scan/scanHelpers';
import { useScanWorkflow } from './scan/useScanWorkflow';

export type ScanAndPreviewViewHandle = { runScan: () => Promise<void> | void };

export const ScanAndPreviewView = forwardRef<ScanAndPreviewViewHandle, {
  client: SweeperClient; settings: Settings | null; settingsVersion?: number; elevationStatus: ElevationStatus; elevationError: string | null;
  onOpenLog: (logId: string) => void; onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}>(function ScanAndPreviewView({ client, settings, settingsVersion, elevationError, onOpenLog, onOpenSettings }, ref) {
  const w = useScanWorkflow({ client, settings, settingsVersion });
  const allow = w.allowDeleteUnknown; const targets = w.visibleBaseTargets; const sel = w.selectedIds;
  const selectableCount = useMemo(() => targets.reduce((n, t) => n + t.items.filter((i) => isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: allow })).length, 0), [targets, allow]);
  const skippedCount = useMemo(() => targets.reduce((n, t) => n + t.items.filter((i) => !isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: allow })).length, 0), [targets, allow]);
  const selectedBytes = useMemo(() => sumDecimalStrings(targets.flatMap((t) => t.items.filter((i) => sel.has(i.id)).map((i) => i.size))), [targets, sel]);
  const requiresConfirmation = useMemo(() => scanSelectionNeedsConfirmation({ targets, selectedIds: sel }), [targets, sel]);
  const eligibleSafeIds = useMemo(() => targets.flatMap((t) => t.risk === 'Safe' ? t.items.filter((i) => isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: allow })).map((i) => i.id) : []), [targets, allow]);
  const allEligibleSelected = eligibleSafeIds.length > 0 && eligibleSafeIds.every((id) => sel.has(id));
  const selectedCategoryCount = useMemo(() => targets.filter((t) => t.items.some((i) => sel.has(i.id))).length, [targets, sel]);
  const heroReclaimable = useMemo(() => targets.reduce((a, t) => a + BigInt(t.eligibleBytes || '0'), 0n).toString(), [targets]);
  const settingsStale = !!(w.results && w.scanSettingsVersion !== null && settingsVersion !== undefined && w.scanSettingsVersion !== settingsVersion);
  const isFilterActive = w.toolbar.filter !== 'all' || w.toolbar.query.trim().length > 0;
  const visibleEligibleSafeIds = useMemo(() => w.visibleTargets.flatMap(({ target, items }) => target.risk === 'Safe' ? items.filter((i) => isSelectableScanItem({ item: i, target, allowDeleteUnknown: allow })).map((i) => i.id) : []), [w.visibleTargets, allow]);
  const selectSafeIds = isFilterActive ? visibleEligibleSafeIds : eligibleSafeIds;
  const allSelectSafeSelected = selectSafeIds.length > 0 && selectSafeIds.every((id) => sel.has(id));
  const canPreview = !!w.sessionId && sel.size > 0 && !w.scanning && !w.previewLoading && !w.cleaning && !w.isSessionExpired && (!requiresConfirmation || w.confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!w.preview && !w.scanning && !w.previewLoading && !w.cleaning;
  const inspectedCtx = useMemo(() => {
    if (!w.focus || w.focus.kind !== 'item' || !w.inspected || w.inspected.itemId !== w.focus.itemId) return null;
    const inspectedId = w.inspected.itemId;
    for (const t of targets) { const it = t.items.find((i) => i.id === inspectedId); if (it) return { target: t, item: it }; }
    return null;
  }, [w.inspected, targets, w.focus]);
  const focusedGroup = useMemo(() => {
    const focus = w.focus;
    if (!focus || focus.kind !== 'group') return null;
    return targets.find((t) => t.targetId === focus.targetId) ?? null;
  }, [w.focus, targets]);
  const cleanProgressLabel = w.cleanProgress ? `Progress: ${w.cleanProgress.completedItems} / ${w.cleanProgress.totalItems} items — ${formatBytes(w.cleanProgress.bytesFreedSoFar)} freed so far (${w.cleanProgress.status})` : null;
  useImperativeHandle(ref, () => ({ runScan: w.runScan }), [w.runScan]);

  const invalidate = () => { w.invalidatePreviewRequest(); w.setPreview(null); w.setCleanSummary(null); w.setCleanProgress(null); };
  const setTargetSelection = (target: TargetScanResultDto, select: boolean) => { invalidate(); w.setSelectedIds((prev) => { const next = new Set(prev); for (const item of target.items) { if (!isSelectableScanItem({ item, target, allowDeleteUnknown: allow })) continue; if (select) next.add(item.id); else next.delete(item.id); } return next; }); };
  const toggleItem = (item: ScanItemDto, target: TargetScanResultDto) => { if (w.cleaning) return; if (!isSelectableScanItem({ item, target, allowDeleteUnknown: allow })) { w.setFocus({ kind: 'item', itemId: item.id }); void w.inspectItem(item.id); return; } invalidate(); w.setSelectedIds((prev) => { const next = new Set(prev); if (next.has(item.id)) next.delete(item.id); else next.add(item.id); return next; }); w.setFocus({ kind: 'item', itemId: item.id }); void w.inspectItem(item.id); };
  const toggleSelectAllEligibleSafe = () => { invalidate(); w.setSelectedIds((prev) => { const next = new Set(prev); if (allEligibleSelected) eligibleSafeIds.forEach((id) => next.delete(id)); else eligibleSafeIds.forEach((id) => next.add(id)); return next; }); };
  const selectAllSafe = () => { invalidate(); w.setSelectedIds((prev) => { const next = new Set(prev); selectSafeIds.forEach((id) => next.add(id)); return next; }); };
  const clearSelection = () => { invalidate(); w.setSelectedIds(new Set()); };
  async function runPreview() {
    if (!w.sessionId) return;
    if (w.isSessionExpired) { w.setError('Scan session expired or was invalidated. Start a new Scan.'); w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.'); return; }
    const requestId = w.activePreviewRequestRef.current + 1; w.activePreviewRequestRef.current = requestId; const requestRevision = w.previewRevisionRef.current; const requestSessionId = w.sessionId;
    w.setError(null); w.setInfo(null); w.setPreviewLoading(true); w.setCleanSummary(null); w.setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: w.confirmText || null } : null;
      const res = await client.previewClean(requestSessionId, Array.from(sel), confirmations);
      if (w.activePreviewRequestRef.current !== requestId || w.previewRevisionRef.current !== requestRevision || w.sessionId !== requestSessionId) return;
      w.setPreview({ ...res, issuedAtMs: Date.now() });
    } catch (e) {
      if (w.activePreviewRequestRef.current !== requestId || w.previewRevisionRef.current !== requestRevision || w.sessionId !== requestSessionId) return;
      const ce = e as ContractError | undefined; w.setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') w.setPreview(null);
    } finally { if (w.activePreviewRequestRef.current === requestId) w.setPreviewLoading(false); }
  }
  async function runClean() {
    if (!w.preview) return; w.setError(null); w.setInfo(null); w.setCleaning(true); w.setCleanProgress(null); w.setCleanSummary(null);
    try {
      const res = await client.executeClean(w.preview.previewToken, (ev) => w.setCleanProgress(ev));
      w.setCleanSummary(res); w.setPreview(null); w.setSelectedIds(new Set()); w.setConfirmText(''); w.setInfo('Clean completed. Review the summary and Activity History below.');
      if (w.sessionId) { try { w.setResults(await client.getScanResults(w.sessionId)); } catch { /* ignore */ } }
    } catch (e) {
      const ce = e as ContractError | undefined; w.setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') { w.setPreview(null); w.setInfo('Preview token is invalid, expired, or already used. Run Preview again.'); }
    } finally { w.setCleaning(false); }
  }
  async function runAsAdministrator() {
    w.setRelaunching(true); w.setRelaunchInfo(null); w.setRelaunchError(null);
    try { const res = await client.relaunchElevated(); w.setRelaunchInfo(res.launched ? 'Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.' : 'Administrator launch was not started.'); }
    catch (e) { w.setRelaunchError(formatContractError(e)); } finally { w.setRelaunching(false); }
  }

  const hasResults = !!w.results; const anyCollapsed = targets.length > 0 && w.collapsedTargets.size > 0; const masterIndeterminate = !allEligibleSelected && sel.size > 0;
  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header scan-header-row" aria-label="Scan and Clean header">
        <div className="page-header-left scan-header-title-block"><div className="page-title-row"><h1 className="page-title" id="scan-heading">Scan &amp; Clean</h1>{w.results && <span className="header-subtitle num" aria-label="Reclaimable total">{formatBytes(heroReclaimable)} reclaimable</span>}</div></div>
        <div className="page-header-right scan-header-actions scan-header-actions-v19">
          <div className="scan-header-scopes scan-header-scopes-inline scan-header-scopes-subtle" aria-label="Scan scopes" title="Scope changes apply on Rescan">
            <span className="scan-scope-label muted">Scan Scope:</span>
            <div className="scan-scope-inline-controls scan-scope-inline-controls-subtle" role="group" aria-label="Scan scopes">
              <span className="scan-scope-inline-item"><span className={`scan-scope-chip scan-scope-pill scan-scope-config-chip${w.tier1 ? ' active' : ''}`}><label className="scan-scope-pill-label"><input className="sr-only" type="checkbox" checked={w.tier1} onChange={(e) => w.setTier1(e.target.checked)} disabled={w.scanning || w.cleaning} aria-label="Temp Files" /><span className="scan-scope-pill-text">{w.tier1 ? '✓ ' : ''}Temp Files</span></label><span className="scan-scope-pill-info"><InfoTip label="About System &amp; App Temp (Tier 1)"><p><strong>System &amp; App Temp (Tier 1)</strong><br />Windows Temp, crash dumps &amp; app caches older than 24h</p></InfoTip></span></span></span>
              <span className="scan-scope-inline-item"><span className={`scan-scope-chip scan-scope-pill scan-scope-config-chip${w.tier2 ? ' active' : ''}`}><label className="scan-scope-pill-label"><input className="sr-only" type="checkbox" checked={w.tier2} onChange={(e) => w.setTier2(e.target.checked)} disabled={w.scanning || w.cleaning} aria-label="Dev Caches" /><span className="scan-scope-pill-text">{w.tier2 ? '✓ ' : ''}Dev Caches</span></label><span className="scan-scope-pill-info"><InfoTip label="About Developer Caches (Tier 2)"><p><strong>Developer Caches (Tier 2)</strong><br />npm, Cargo, pip &amp; build caches that rebuild automatically. Scope changes apply on the next Rescan. Matches <strong>Settings → General</strong>.</p></InfoTip></span></span></span>
              {onOpenSettings ? <button type="button" className={`scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip${settings?.enableTier3 ? ' active' : ''}`} onClick={() => onOpenSettings('enableTier3')} aria-label={settings?.enableTier3 ? 'Deep Cleanup: On · Settings' : 'Deep Cleanup: Off · Configure'} title={settings?.enableTier3 ? 'Deep Cleanup is on (configure in Settings)' : 'Configure Deep Cleanup in Settings'}><span className="scan-scope-pill-label scan-scope-pill-label-static"><span className="scan-scope-pill-text">{settings?.enableTier3 ? 'Deep Cleanup: On' : 'Deep Cleanup: Off'} <span aria-hidden="true">⚙</span></span></span></button> : <span className={`scan-scope-chip scan-scope-pill scan-deep-cleanup-scope-pill scan-scope-config-chip ${settings?.enableTier3 ? 'active on' : 'off'}`} aria-label={`Deep Cleanup: ${settings?.enableTier3 ? 'On' : 'Off'}`}><span className="scan-scope-pill-label scan-scope-pill-label-static"><span className="scan-scope-pill-text">Deep Cleanup: {settings?.enableTier3 ? 'On' : 'Off'} <span aria-hidden="true">⚙</span></span></span></span>}
            </div>
          </div>
          <div className="scan-header-rescan-wrap">{settingsStale && <span className="settings-updated-pill" role="status">Settings updated — Rescan to apply</span>}<button key={hasResults ? 'rescan' : 'scan'} className={hasResults ? 'btn secondary-action scan-rescan-btn' : 'btn primary scan-rescan-btn'} onClick={() => void w.runScan()} disabled={w.scanning || w.cleaning} aria-label="Start Scan">{hasResults ? <><RefreshIcon aria-hidden="true" /> <span>Rescan</span></> : 'Start Scan'}</button></div>
        </div>
      </header>
      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {w.relaunchInfo && <div className="notice info" role="status">{w.relaunchInfo}</div>}
      {w.relaunchError && <div className="notice error" role="alert">{w.relaunchError}</div>}
      {w.error && <div className="notice error" role="alert">{w.error}</div>}
      {w.info && <div className="notice info" role="status">{w.info}</div>}
      <section className="scan-controls scan-controls-compact" aria-label="Scan controls">
        <div className="scan-table-toolbar scan-table-toolbar-mainbar" aria-label="Scan table toolbar">
          <div className="scan-table-toolbar-left"><div className="segmented-toggle scan-status-segmented" role="group" aria-label="Status filters"><button type="button" className={`seg-btn ${w.toolbar.filter === 'all' ? 'active' : ''}`} aria-pressed={w.toolbar.filter === 'all'} onClick={() => w.setToolbar({ ...w.toolbar, filter: 'all' })}>All ({targets.length} categories · {selectableCount + skippedCount} files)</button><button type="button" className={`seg-btn ${w.toolbar.filter === 'eligible' ? 'active' : ''}`} aria-pressed={w.toolbar.filter === 'eligible'} aria-label={`Eligible (${selectableCount} files)`} onClick={() => w.setToolbar({ ...w.toolbar, filter: 'eligible' })}>Safe ({selectableCount})</button><button type="button" className={`seg-btn ${w.toolbar.filter === 'skipped' ? 'active' : ''}`} aria-pressed={w.toolbar.filter === 'skipped'} onClick={() => w.setToolbar({ ...w.toolbar, filter: 'skipped' })}>Skipped ({skippedCount})</button></div></div>
          <div className="scan-table-toolbar-middle"><input type="search" className="scan-files-search" placeholder="Search files..." value={w.toolbar.query} onChange={(e) => w.setToolbar({ ...w.toolbar, query: e.target.value })} aria-label="Search files" disabled={!w.results} /></div>
          <div className="scan-table-toolbar-right"><label className="toolbar-group scan-sort-group">Sort<select value={w.toolbar.sort} onChange={(e) => w.setToolbar({ ...w.toolbar, sort: e.target.value as 'size' | 'name' })} aria-label="Sort scan results" disabled={!w.results}><option value="size">Size</option><option value="name">Name</option></select></label><button className="btn small subtle-btn" onClick={() => w.setCollapsedTargets(anyCollapsed ? new Set() : new Set(targets.map((t) => t.targetId)))} disabled={w.cleaning || !w.results}>{anyCollapsed ? 'Expand All' : 'Collapse All'}</button><button className="btn small" onClick={selectAllSafe} disabled={w.cleaning || selectSafeIds.length === 0} aria-label={isFilterActive ? `Select Visible Safe (${selectSafeIds.length})` : 'Select Safe'} title={allSelectSafeSelected ? 'All safe items selected' : undefined}>{allSelectSafeSelected ? `✓ ${isFilterActive ? `Select Visible Safe (${selectSafeIds.length})` : 'Select Safe'}` : (isFilterActive ? `Select Visible Safe (${selectSafeIds.length})` : 'Select Safe')}</button></div>
        </div>
      </section>
      {w.scanning && <div role="status" aria-live="polite" className="notice info scanning-pulse scan-progress-notice"><div className="scan-progress-header"><span className="scan-progress-status">{w.scanStatus || 'Scanning…'}</span>{w.scanProgress && w.scanProgress.totalTargets > 0 && <span className="scan-progress-targets num" aria-label="Scan target progress">{w.scanProgress.completedTargets} / {w.scanProgress.totalTargets} Targets</span>}</div>{w.scanProgress && <div className="scan-progress-meta">{w.scanProgress.currentTargetName && <span className="scan-progress-current">Current: <span className="scan-progress-current-name">{w.scanProgress.currentTargetName}</span></span>}<span className="scan-progress-counts num">{(() => { try { return BigInt(w.scanProgress.scannedItems || '0').toLocaleString(); } catch { return w.scanProgress.scannedItems || '0'; } })()} items · {formatBytes(w.scanProgress.scannedBytes || '0')}</span></div>}<div className="progress">{w.scanProgress && w.scanProgress.totalTargets > 0 ? <progress className="scan-progress-meter" value={w.scanProgress.completedTargets} max={w.scanProgress.totalTargets} aria-label="Scan progress" /> : <div className="progress-bar progress-indeterminate" aria-hidden="true" />}</div></div>}
      {!w.scanning && w.scanStatus && <div className={`notice ${w.scanStatus.toLowerCase().includes('failed') || w.scanStatus.toLowerCase().includes('cancelled') ? 'error' : 'info'}`} role={w.scanStatus.toLowerCase().includes('failed') || w.scanStatus.toLowerCase().includes('cancelled') ? 'alert' : 'status'}>{w.scanStatus}</div>}
      {!w.results && !w.scanning && <section className="empty-state" aria-label="Scan onboarding"><h3>Ready to scan</h3><p>Choose scope above, then run <strong>Start Scan</strong>. Nothing is removed until you Preview and confirm <strong>Clean Permanently</strong>.</p></section>}
      {w.scanning && !w.results && <section className="master-detail scan-layout" aria-label="Scanning"><div className="master-pane"><div className="scan-table-header"><span className="scan-th-item"><label className="checkbox scan-master-check" aria-label={allEligibleSelected ? 'Deselect all eligible files' : 'Select all eligible files'}><input type="checkbox" checked={allEligibleSelected} aria-checked={masterIndeterminate ? 'mixed' : allEligibleSelected} ref={(el) => { if (el) el.indeterminate = masterIndeterminate; }} disabled={eligibleSafeIds.length === 0 || w.scanning || w.cleaning} onChange={toggleSelectAllEligibleSafe} aria-label={allEligibleSelected ? 'Deselect all' : 'Select all eligible'} /></label><span>Item</span></span><span className="scan-th-items num">Items</span><span className="scan-th-size num">Size</span><span className="scan-th-status">Status</span></div><div className="pane-body flush" aria-hidden="true"><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /></div></div><div className="detail-pane selection-pane scan-inspector-pane"><div className="pane-header"><h3>Inspector</h3></div><div className="detail-scroll"><p className="muted">Scanning for reclaimable space…</p></div></div></section>}
      {w.results && (
        <>
          <section className="master-detail scan-layout scan-layout-with-bar" aria-label="Scan results and review">
            <div className="master-pane scan-master-pane">
              <div className="scan-table-header"><span className="scan-th-item"><label className="checkbox scan-master-check" aria-label={allEligibleSelected ? 'Deselect all eligible files' : 'Select all eligible files'}><input type="checkbox" checked={allEligibleSelected} aria-checked={masterIndeterminate ? 'mixed' : allEligibleSelected} ref={(el) => { if (el) el.indeterminate = masterIndeterminate; }} disabled={eligibleSafeIds.length === 0 || w.scanning || w.cleaning} onChange={toggleSelectAllEligibleSafe} aria-label={allEligibleSelected ? 'Deselect all' : 'Select all eligible'} /></label><span>Item</span></span><span className="scan-th-items num">Items</span><span className="scan-th-size num">Size</span><span className="scan-th-status">Status</span></div>
              <div className="pane-body flush scan-results-body scan-results-shell">
                {targets.length === 0 && <div className="empty-state"><h3>No targets</h3><p>No Targets returned for this Scan.</p></div>}
                <div className="target-list scan-grid-list scan-table-scroll">
                  {w.visibleTargets.map(({ target, items }) => {
                    const groupActive = w.focus?.kind === 'group' ? w.focus.targetId === target.targetId : false;
                    return (
                    <ScanTargetCard key={target.targetId} target={target} visibleItems={items} allowDeleteUnknown={allow} minAgeHours={w.effectiveMinAgeHours} selectedIds={sel} collapsed={w.collapsedTargets.has(target.targetId)} showAll={w.showAllGroups.has(target.targetId)} focusedItemId={w.focus?.kind === 'item' ? w.focus.itemId : null} groupActive={groupActive} cleaning={w.cleaning} onFocusGroup={() => w.setFocus({ kind: 'group', targetId: target.targetId })} onToggleCollapsed={() => w.setCollapsedTargets((prev) => { const next = new Set(prev); if (next.has(target.targetId)) next.delete(target.targetId); else next.add(target.targetId); return next; })} onToggleGroupSelect={() => setTargetSelection(target, !(target.items.filter((i) => isSelectableScanItem({ item: i, target, allowDeleteUnknown: allow })).every((i) => sel.has(i.id))))} onToggleItem={(item) => toggleItem(item, target)} onToggleShowAll={() => w.setShowAllGroups((prev) => { const next = new Set(prev); if (next.has(target.targetId)) next.delete(target.targetId); else next.add(target.targetId); return next; })} onOpenSettings={onOpenSettings} />
                    );
                  })}
                </div>
              </div>
            </div>
            <ScanInspectorPanel focus={w.focus} focusedGroup={focusedGroup} inspected={w.inspected} inspectedCtx={inspectedCtx} allowDeleteUnknown={allow} minAgeHours={w.effectiveMinAgeHours} selectedIds={sel} requiresConfirmation={requiresConfirmation} confirmText={w.confirmText} onConfirmText={w.setConfirmText} cleaning={w.cleaning} preview={w.preview} cleanProgressLabel={cleanProgressLabel} cleanSummary={w.cleanSummary} onOpenLog={onOpenLog} onOpenSettings={onOpenSettings} />
          </section>
          <ScanCleanBar selectedIdsCount={sel.size} selectedCategoryCount={selectedCategoryCount} selectedBytes={selectedBytes} cleaning={w.cleaning} previewLoading={w.previewLoading} hasPreview={!!w.preview} canPreview={canPreview} canClean={canClean} onClear={clearSelection} onPreview={() => void runPreview()} onClean={() => void runClean()} />
        </>
      )}
    </div>
  );
});
