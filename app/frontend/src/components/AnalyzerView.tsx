import { formatBytes, normalizeDisplayPath, pluralize } from '../utils/format';
import { TYPED_CONFIRMATION_PHRASE } from './cleanShared';
import { BrowseIcon, RefreshIcon } from './Icons';
import { AnalyzerCleanPanel } from './analyzer/AnalyzerCleanPanel';
import { AnalyzerInspectorPanel } from './analyzer/AnalyzerInspectorPanel';
import { AnalyzerTreeRow } from './analyzer/AnalyzerTreeRow';
import { ALLOC_COLORS, BROWSE_PATHS, isMoreRowId, isNodeSelectable } from './analyzer/analyzerHelpers';
import type { StartAnalyzerResult } from '../api/client';
import { useAnalyzerWorkflow } from './analyzer/useAnalyzerWorkflow';
import type { Settings, SweeperClient } from '../api/client';

type AllocSeg = { id: string; name: string; bytes: bigint; pct: number; pctVisual: number; color: string; isOther?: boolean; focusNodeId?: string };
function buildAllocSegments(result: StartAnalyzerResult | null, rootTotalBytes: bigint): AllocSeg[] {
  if (!result) return [];
  const kids = [...(result.root.children ?? [])].sort((a, b) => {
    const sa = BigInt(a.sizeBytes || '0');
    const sb = BigInt(b.sizeBytes || '0');
    return sa === sb ? 0 : (sa > sb ? -1 : 1);
  });
  const major: AllocSeg[] = [];
  let otherBytes = 0n;
  kids.forEach((k, i) => {
    const bytes = BigInt(k.sizeBytes || '0');
    const pct = rootTotalBytes > 0n ? Number((bytes * 10000n) / rootTotalBytes) / 100 : 0;
    if (pct >= 0.1) major.push({ id: k.id, name: k.name, bytes, pct, pctVisual: 0, color: ALLOC_COLORS[i % ALLOC_COLORS.length], focusNodeId: k.id });
    else otherBytes += bytes;
  });
  const raw = [...major];
  if (otherBytes > 0n) {
    const pct = rootTotalBytes > 0n ? Number((otherBytes * 10000n) / rootTotalBytes) / 100 : 0;
    raw.push({ id: 'alloc-other', name: 'Other', bytes: otherBytes, pct, pctVisual: 0, color: '#64748b', isOther: true });
  }
  const visible = raw.filter((s) => s.pct > 0);
  const gapPct = visible.length > 1 ? 0.2 : 0;
  const targetSum = Math.max(0, 100 - gapPct * Math.max(0, visible.length - 1));
  const minTiny = 0.35;
  let adjusted = raw.map((s) => ({ ...s, pctVisual: s.pct <= 0 ? 0 : Math.max(minTiny, s.pct) }));
  let sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
  if (sum > targetSum) {
    const reducible = adjusted.filter((s) => s.pctVisual > minTiny);
    const room = reducible.reduce((a, b) => a + (b.pctVisual - minTiny), 0);
    if (room > 0) adjusted = adjusted.map((s) => s.pctVisual <= minTiny ? s : { ...s, pctVisual: s.pctVisual - ((sum - targetSum) * (s.pctVisual - minTiny)) / room });
  }
  sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
  if (sum < targetSum && adjusted.length > 0) {
    const largest = [...adjusted].sort((a, b) => b.pct - a.pct)[0];
    adjusted = adjusted.map((s) => s.id === largest.id ? { ...s, pctVisual: s.pctVisual + (targetSum - sum) } : s);
  }
  return adjusted;
}

function AllocBar(props: { segments: AllocSeg[]; result: StartAnalyzerResult; displayPath: string; rootPath: string; analyzing: boolean; cleaning: boolean; onRun: (path: string) => void; onFocus: (id: string) => void }) {
  const { segments, result } = props;
  const vbW = 1000;
  const visible = segments.filter((s) => s.pctVisual > 0);
  const gapUnits = visible.length > 1 ? 1 : 0;
  const available = Math.max(0, vbW - gapUnits * Math.max(0, visible.length - 1));
  let widths = segments.map((s) => (s.pctVisual > 0 ? (s.pctVisual / 100) * vbW : 0));
  const widthSum = widths.reduce((a, b) => a + b, 0);
  if (widthSum > 0) widths = widths.map((w) => (w * available) / widthSum);
  const minSegW = 4;
  const sizeableIdx = widths.map((w, i) => (w > 0 ? i : -1)).filter((i) => i >= 0);
  let extra = 0;
  widths = widths.map((w) => { if (w <= 0) return 0; const clamped = Math.max(minSegW, w); extra += clamped - w; return clamped; });
  if (extra > 0 && sizeableIdx.length > 0) {
    const reducible = sizeableIdx.map((i) => Math.max(0, widths[i] - minSegW));
    const reducibleTotal = reducible.reduce((a, b) => a + b, 0);
    if (reducibleTotal > 0) sizeableIdx.forEach((idx, j) => { widths[idx] -= Math.min(reducible[j], (extra * reducible[j]) / reducibleTotal); });
    const finalSum = widths.reduce((a, b) => a + b, 0);
    if (finalSum > available && finalSum > 0) widths = widths.map((w) => (w * available) / finalSum);
  }
  let x = 0;
  let renderedVisible = 0;
  const isAtRoot = /^([A-Za-z]:\\)$/.test(props.displayPath);
  const breadcrumbParts = props.displayPath.split('\\').filter(Boolean).map((seg, i, arr) => ({ label: i === 0 ? `${seg}\\` : seg, path: arr.slice(0, i + 1).join('\\') + (i === 0 ? '\\' : '') }));
  return (
    <section className="alloc-bar-wrap" aria-label="Analyzer allocation">
      <div className="alloc-bar-header">
        <div className="alloc-bar-header-left">
          <span className="alloc-hotspots-label">Hotspots under {props.displayPath} · Sorted by size</span>
          {!isAtRoot && (
            <nav className="analyzer-breadcrumb analyzer-breadcrumb-trail alloc-breadcrumb-trail" aria-label="Analyzed path breadcrumb">
              {breadcrumbParts.map((part, i) => (
                <span key={part.path} className="breadcrumb-seg">
                  {i > 0 && <span className="breadcrumb-sep" aria-hidden="true">&gt;</span>}
                  <button type="button" className="breadcrumb-btn" onClick={() => props.onRun(part.path)} disabled={props.analyzing || props.cleaning}>{part.label.replace(/\\$/, '')}</button>
                </span>
              ))}
            </nav>
          )}
        </div>
        <span className="alloc-bar-header-meta muted num">{formatBytes(result.totalBytes)} · Top consumers</span>
      </div>
      <svg className="alloc-bar" viewBox="0 0 1000 14" preserveAspectRatio="none" role="img" aria-label="Top-level folder allocation">
        {segments.map((s, i) => {
          const w = Math.max(0, widths[i] ?? 0);
          if (w <= 0) return null;
          const isLastVisible = renderedVisible === visible.length - 1;
          const gap = !isLastVisible && visible.length > 1 ? gapUnits : 0;
          const clickable = !!s.focusNodeId;
          const rectW = Math.min(w, Math.max(0, vbW - x - gap));
          const rect = <rect key={s.id} x={x} y={0} width={Math.max(0, rectW)} height={14} fill={s.color} className={`alloc-segment hotspot-seg${clickable ? '' : ' alloc-segment-static'}`} onClick={clickable ? () => props.onFocus(s.focusNodeId!) : undefined}><title>{`${s.name}: ${formatBytes(s.bytes.toString())} (${formatSharePercent(s.pct)})`}</title></rect>;
          x += rectW + gap;
          renderedVisible += 1;
          return rect;
        })}
      </svg>
      <div className="alloc-legend">
        {segments.map((s) => {
          const labelPct = s.name === 'Other' && s.pct < 0.1 ? '<0.1%' : formatSharePercent(s.pct);
          const body = (<><svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><rect x="0" y="0" width="8" height="8" fill={s.color} rx="2" /></svg><span>{s.name}</span><span className="muted num">{formatBytes(s.bytes.toString())} · {labelPct}</span></>);
          return s.focusNodeId ? <button key={s.id} type="button" className="alloc-legend-item alloc-legend-btn" onClick={() => props.onFocus(s.focusNodeId!)} title={`Inspect ${s.name}`}>{body}</button> : <span key={s.id} className="alloc-legend-item alloc-legend-static" aria-label={`${s.name} ${labelPct}`}>{body}</span>;
        })}
      </div>
    </section>
  );
}

function formatSharePercent(share: number): string {
  if (share <= 0) return '0%';
  if (share < 0.1) return '<0.1%';
  const pct1 = Math.floor(share * 10) / 10;
  return `${pct1.toFixed(1)}%`;
}

export function AnalyzerView({ client, settings, onOpenLog, onOpenSettings }: { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void; onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void }) {
  const w = useAnalyzerWorkflow({ client, settings });
  const allocSegments = buildAllocSegments(w.result, w.rootTotalBytes);
  const displayPath = normalizeDisplayPath((w.analyzedPath || w.rootPath).trim() || 'C:\\');
  const visibleExpandableIds = (() => {
    if (!w.filteredRoot) return [] as string[];
    const ids: string[] = [];
    const walk = (n: { id: string; isDir: boolean; children?: { id: string; isDir: boolean; children?: unknown[] }[] }, isRoot: boolean) => {
      if (!isRoot && n.isDir && (n.children?.length ?? 0) > 0) ids.push(n.id);
      for (const c of n.children ?? []) walk(c as { id: string; isDir: boolean; children?: { id: string; isDir: boolean; children?: unknown[] }[] }, false);
    };
    for (const c of w.filteredRoot.children ?? []) walk(c as never, false);
    return ids;
  })();
  const allVisibleExpanded = visibleExpandableIds.length > 0 && visibleExpandableIds.every((id) => w.expanded.has(id));
  const visibleSelectableIds = w.flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, w.allowDeleteUnknown)).map((r) => r.node.id);
  const allVisibleSelectableSelected = visibleSelectableIds.length > 0 && visibleSelectableIds.every((id) => w.selectedIds.has(id));
  const visibleSelectedCount = visibleSelectableIds.filter((id) => w.selectedIds.has(id)).length;
  const hasResult = !!w.result;
  const hasSelectedRisky = w.selectedNodes.some((n) => n.risk === 'Caution' || n.risk === 'Unknown');
  const cleanProgressLabel = w.cleanProgress ? `Progress: ${w.cleanProgress.completedItems} / ${w.cleanProgress.totalItems} items — ${formatBytes(w.cleanProgress.bytesFreedSoFar)} freed so far (${w.cleanProgress.status})` : null;
  const inspectedNode = w.inspected ? w.nodesById.get(w.inspected.itemId) ?? null : null;
  const canPreview = !!w.sessionId && w.selectedIds.size > 0 && !w.analyzing && !w.previewLoading && !w.cleaning && (!w.requiresConfirmation || w.confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!w.preview && !w.analyzing && !w.previewLoading && !w.cleaning;
  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header" aria-label="Analyzer header">
        <div className="page-header-left analyzer-header-left">
          <div className="page-title-row">
            <h1 className="page-title" id="analyzer-heading">Analyzer</h1>
            <span className="header-subtitle num" aria-label="Analyzer subtitle">{w.result ? `${displayPath} · ${formatBytes(w.result.totalBytes)} across ${pluralize((w.result.root.children ?? []).length, 'entry', 'entries')} (${Number(w.result.totalItems || '0')} files total) · All files on disk` : `All files & disk hotspots under ${displayPath}`}</span>
          </div>
        </div>
        <div className="page-header-right">
          <button key={hasResult ? 'reanalyze' : 'analyze'} className={hasResult ? 'btn secondary-action' : 'btn primary'} onClick={() => void w.runAnalyzer()} disabled={w.analyzing || w.cleaning || !w.rootPath.trim()} aria-label="Analyze">{hasResult ? <><RefreshIcon aria-hidden="true" /> <span>Re-analyze</span></> : 'Analyze'}</button>
        </div>
      </header>
      {w.error && <div className="notice error" role="alert">{w.error}</div>}
      {w.info && <div className="notice info" role="status">{w.info}</div>}
      <div className="analyzer-location-bar" aria-label="Analyzer location bar">
        <div className="analyzer-root-group analyzer-root-group-tight">
          <label htmlFor="analyzer-root" className="muted small-note">Root</label>
          <div className="analyzer-root-inline"><input id="analyzer-root" type="text" value={w.rootPath} onChange={(e) => w.setRootPath(e.target.value)} aria-label="Analyzer root path" disabled={w.analyzing || w.cleaning} className="analyzer-root-input" /><button type="button" className="btn small analyzer-go-btn" onClick={() => void w.runAnalyzer()} disabled={w.analyzing || w.cleaning || !w.rootPath.trim()} aria-label="Go">Go</button></div>
          <button type="button" className="btn browse-btn" onClick={w.cycleBrowsePath} disabled={w.analyzing || w.cleaning} aria-label="Use preset root path"><BrowseIcon /> <span>Use Preset</span></button>
        </div>
        <div className="quick-roots quick-roots-inline" aria-label="Quick root path presets"><span className="muted small-note">Quick:</span>{(['C:\\', 'C:\\Users', 'Temp'] as const).filter((label) => { const value = label === 'Temp' ? 'C:\\Windows\\Temp' : label; const norm = (p: string) => p.trim().replace(/[\\/]+$/, '').toLowerCase(); return !(label === 'C:\\' && norm(w.rootPath) === norm('C:\\')); }).map((label) => { const value = label === 'Temp' ? 'C:\\Windows\\Temp' : label; return <button key={label} type="button" className="chip-btn pill-chip small-chip" onClick={() => { w.setRootPath(value); if (hasResult) void w.runAnalyzer(value); }} disabled={w.analyzing || w.cleaning}>{label}</button>; })}</div>
      </div>
      {w.result && allocSegments.length > 0 && <AllocBar segments={allocSegments} result={w.result} displayPath={displayPath} rootPath={w.rootPath} analyzing={w.analyzing} cleaning={w.cleaning} onRun={(path) => void w.runAnalyzer(path)} onFocus={w.focusNode} />}
      {w.analyzing && (
        <div role="status" aria-live="polite" className="notice info scanning-pulse analyzer-progress-notice">
          <div className="analyzer-progress-row"><span className="analyzer-progress-label">Analyzing…</span><span className="analyzer-progress-stats num">{w.progress ? `dirs ${w.progress.scannedDirs}, files ${w.progress.scannedFiles}, ${formatBytes(w.progress.scannedBytes)}` : 'Starting…'}</span></div>
          {w.progress?.currentPath && <div className="analyzer-progress-path-row"><span className="analyzer-progress-path-prefix muted">Current:</span><span className="analyzer-progress-path mono" title={normalizeDisplayPath(w.progress.currentPath)}>{normalizeDisplayPath(w.progress.currentPath)}</span></div>}
          <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
        </div>
      )}
      {w.result && (
        <>
          <section className="master-detail analyzer-layout scan-layout-with-bar" aria-label="Analyzer results">
            <div className="master-pane">
              <h2 id="analyzer-tree-heading" className="sr-only">Analyzer Tree</h2>
              <div className="analyzer-table-toolbar analyzer-table-toolbar-compact analyzer-toolbar" aria-label="Analyzer table filters">
                <div className="analyzer-toolbar-row analyzer-toolbar-row-top">
                  <div className="segmented-toggle scan-status-segmented analyzer-status-segmented analyzer-status-segmented-compact" role="group" aria-label="Analyzer status filters">
                    <button type="button" className={`seg-btn ${w.statusFilter === 'all' ? 'active' : ''}`} aria-pressed={w.statusFilter === 'all'} onClick={() => w.setStatusFilter('all')}>All ({w.statusCounts.all})</button>
                    <button type="button" className={`seg-btn ${w.statusFilter === 'safe' ? 'active' : ''}`} aria-pressed={w.statusFilter === 'safe'} onClick={() => w.setStatusFilter('safe')}>Safe ({w.statusCounts.safe})</button>
                    <button type="button" className={`seg-btn ${w.statusFilter === 'caution' ? 'active' : ''}`} aria-pressed={w.statusFilter === 'caution'} onClick={() => w.setStatusFilter('caution')}>Caution ({w.statusCounts.caution})</button>
                    <button type="button" className={`seg-btn ${w.statusFilter === 'unknown' ? 'active' : ''}`} aria-pressed={w.statusFilter === 'unknown'} onClick={() => w.setStatusFilter('unknown')}>Unknown ({w.statusCounts.unknown})</button>
                    <button type="button" className={`seg-btn ${w.statusFilter === 'locked' ? 'active' : ''}`} aria-pressed={w.statusFilter === 'locked'} onClick={() => w.setStatusFilter('locked')}>Protected &amp; Read-Only ({w.statusCounts.locked})</button>
                  </div>
                </div>
                <div className="analyzer-toolbar-row analyzer-toolbar-row-actions">
                  <input type="search" className="analyzer-search-input analyzer-search-compact" placeholder="Search folders..." value={w.query} onChange={(e) => w.setQuery(e.target.value)} aria-label="Search analyzer folders" disabled={!w.result} />
                  <div className="analyzer-table-toolbar-actions">
                    <label className="toolbar-group analyzer-sort-group">Sort<select value={w.sort} onChange={(e) => w.setSort(e.target.value as 'size' | 'items' | 'name')} aria-label="Sort analyzer" disabled={!w.result}><option value="size">Size</option><option value="items">Items</option><option value="name">Name</option></select></label>
                    <button type="button" className="btn small" onClick={w.selectSafeAnalyzer} disabled={!w.result || w.analyzing || w.cleaning} aria-label="Select Safe">Select Safe</button>
                    <button type="button" className="btn small subtle-btn" onClick={() => w.toggleExpandCollapseAll(visibleExpandableIds)} disabled={w.analyzing || w.cleaning || !w.result}>{allVisibleExpanded ? 'Collapse All' : 'Expand All'}</button>
                  </div>
                </div>
              </div>
              <div className="analyzer-col-headers" aria-hidden="true">
                <span className="analyzer-col-h-name"><span className="analyzer-header-master-slot"><label className="checkbox analyzer-master-check" aria-label={allVisibleSelectableSelected ? 'Deselect all visible selectable nodes' : 'Select all visible selectable nodes'}><input type="checkbox" checked={allVisibleSelectableSelected} ref={(el) => { if (el) el.indeterminate = visibleSelectedCount > 0 && visibleSelectedCount < visibleSelectableIds.length; }} disabled={w.analyzing || w.cleaning || visibleSelectableIds.length === 0} onChange={() => { w.invalidatePreviewRequest(); w.setPreview(null); w.setCleanSummary(null); w.setCleanProgress(null); w.setSelectedIds((prev: Set<string>) => { const next = new Set(prev); if (allVisibleSelectableSelected) visibleSelectableIds.forEach((id) => next.delete(id)); else visibleSelectableIds.forEach((id) => next.add(id)); return next; }); }} /></label></span><span>Name</span></span>
                <span className="analyzer-col-h-share">Share</span><span className="analyzer-col-h-items">Files</span><span className="analyzer-col-h-size">Size</span><span className="analyzer-col-h-status">Status</span>
              </div>
              <div className="pane-body flush">
                <div className="analyzer-viewport" role="tree" aria-label="Analyzer results tree" tabIndex={0}>
                  {w.flatRows.map((row) => <AnalyzerTreeRow key={row.node.id} row={row} rootTotalBytes={w.rootTotalBytes} expanded={w.expanded} selectedIds={w.selectedIds} inspectedId={w.inspected?.itemId ?? null} allowDeleteUnknown={w.allowDeleteUnknown} cleaning={w.cleaning} onToggleExpand={w.toggleExpand} onToggleSelect={w.toggleSelect} onInspect={(id) => void w.inspectNode(id)} onShowMore={(parentId) => w.setShowAllAnalyzerNodes((prev: Set<string>) => new Set([...prev, parentId]))} onDrill={(path) => { w.setRootPath(path); void w.runAnalyzer(path); }} />)}
                  {w.flatRows.length === 0 && <div className="muted analyzer-empty">No nodes.</div>}
                </div>
              </div>
            </div>
            <AnalyzerInspectorPanel inspected={w.inspected} inspectedNode={inspectedNode} allowDeleteUnknown={w.allowDeleteUnknown} requiresConfirmation={w.requiresConfirmation} confirmText={w.confirmText} onConfirmText={w.setConfirmText} preview={w.preview} cleaning={w.cleaning} cleanProgressLabel={cleanProgressLabel} cleanSummary={w.cleanSummary} onOpenLog={onOpenLog} onOpenSettings={onOpenSettings} />
          </section>
          <AnalyzerCleanPanel selectedCount={w.selectedIds.size} selectedBytes={w.selectedBytes} hasPreview={!!w.preview} hasSelectedRisky={hasSelectedRisky} cleaning={w.cleaning} previewLoading={w.previewLoading} canPreview={canPreview} canClean={canClean} onClear={() => { w.invalidatePreviewRequest(); w.setSelectedIds(new Set()); w.setPreview(null); }} onPreview={() => void w.runPreview()} onClean={() => void w.runClean()} />
        </>
      )}
    </div>
  );
}
