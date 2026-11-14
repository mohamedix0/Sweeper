import { useMemo, useState } from 'react';
import {
  formatContractError,
  type AnalyzerProgressEvent,
  type CleanProgressEvent,
  type ExecuteCleanResponse,
  type Settings,
  type StartAnalyzerResult,
  type SweeperClient,
} from '../api/client';
import type { AnalyzerNodeDto, ContractError, RiskDto } from '../contract/bindings';
import { formatBytes, normalizeDisplayPath, pluralize } from '../utils/format';
import {
  ExplainDrawer,
  TYPED_CONFIRMATION_PHRASE,
  riskClass,
  type ExplainState,
} from './cleanShared';
import { BrowseIcon, ChevronDownIcon, ChevronRightIcon, CopyIcon, LockIcon, RefreshIcon } from './Icons';

type FlatAnalyzerRow = { node: AnalyzerNodeDto; depth: number; siblingMax: bigint };

function buildFlat(root: AnalyzerNodeDto, expanded: Set<string>): FlatAnalyzerRow[] {
  const rows: FlatAnalyzerRow[] = [];
  const walk = (node: AnalyzerNodeDto, depth: number, siblingMax: bigint) => {
    rows.push({ node, depth, siblingMax });
    if (node.isDir && expanded.has(node.id) && node.children && node.children.length > 0) {
      const kids = node.children;
      let max = 0n;
      for (const c of kids) { const s = BigInt(c.sizeBytes || '0'); if (s > max) max = s; }
      if (max === 0n) max = 1n;
      for (const c of kids) walk(c, depth + 1, max);
    }
  };
  walk(root, 0, BigInt(root.sizeBytes || '0') || 1n);
  return rows;
}

function filterTree(node: AnalyzerNodeDto, query: string, sort: 'size' | 'items' | 'name'): AnalyzerNodeDto | null {
  const q = query.trim().toLowerCase();
  const matches = !q || node.name.toLowerCase().includes(q) || node.path.toLowerCase().includes(q);
  let children = (node.children ?? []).map((c) => filterTree(c, query, sort)).filter(Boolean) as AnalyzerNodeDto[];
  children = [...children].sort((a, b) => {
    if (sort === 'name') return a.name.localeCompare(b.name);
    if (sort === 'items') return Number(BigInt(b.itemCount || '0') - BigInt(a.itemCount || '0'));
    const sa = BigInt(a.sizeBytes || '0'), sb = BigInt(b.sizeBytes || '0');
    return sa === sb ? 0 : (sa > sb ? -1 : 1);
  });
  if (!matches && children.length === 0) return null;
  return { ...node, children: matches ? children : children };
}

function analyzerSelectionNeedsConfirmation(nodes: AnalyzerNodeDto[]): boolean {
  return nodes.some((n) => n.risk === 'Unknown' || !n.targetId);
}

function isNodeSelectable(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): boolean {
  if (node.displayOnly || node.isLinkPoint || node.isCloudPlaceholder) return false;
  if (node.risk === 'Protected') return false;
  if (node.path.toLowerCase().endsWith('.vhdx')) return false;
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return !!node.selectable;
}

function analyzerStatusText(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): { text: string; locked: boolean; unknownGated: boolean } {
  if (selectable) {
    return { text: (node.risk === 'Unknown' || !node.targetId) ? 'Selectable (typed confirmation required)' : 'Selectable', locked: false, unknownGated: false };
  }
  if (node.risk === 'Protected') return { text: 'Protected. Not selectable.', locked: true, unknownGated: false };
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return { text: 'Unknown gated by Settings (Allow deleting unknown folders).', locked: true, unknownGated: true };
  if (node.isLinkPoint) return { text: 'Link Point. Not selectable.', locked: true, unknownGated: false };
  if (node.isCloudPlaceholder) return { text: 'Cloud placeholder (Skipped). Not selectable.', locked: true, unknownGated: false };
  if (node.path.toLowerCase().endsWith('.vhdx')) return { text: '.vhdx display-only. Not selectable.', locked: true, unknownGated: false };
  if (node.displayOnly) return { text: 'Display-only. Not selectable.', locked: true, unknownGated: false };
  if (node.reason) return { text: `${node.reason}. Not selectable.`, locked: true, unknownGated: false };
  return { text: 'Not selectable.', locked: true, unknownGated: false };
}

function guardBadge(node: AnalyzerNodeDto): string | null {
  if (node.risk === 'Protected') return 'Protected';
  if (node.path.toLowerCase().endsWith('.vhdx')) return 'Read-Only';
  if (node.isLinkPoint || node.isCloudPlaceholder || node.displayOnly) return 'Read-Only';
  if (node.risk === 'Unknown') return 'Unknown';
  if (node.risk === 'Caution') return 'Caution';
  return null;
}

function nodeTypeTag(node: AnalyzerNodeDto): string | null {
  if (node.path.toLowerCase().endsWith('.vhdx')) return 'Virtual Disk';
  if (node.isLinkPoint) return 'Symlink';
  if (node.isCloudPlaceholder) return 'Cloud';
  return null;
}

function tierLabel(tier: string | null): string | null {
  if (tier === 'Tier1') return 'App Temp';
  if (tier === 'Tier2') return 'Dev Cache';
  if (tier === 'Tier3') return 'Deep Cleanup';
  return null;
}

async function copyText(text: string) {
  try { await navigator.clipboard?.writeText(text); } catch { /* ignore */ }
}

const ALLOC_COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#8b5cf6', '#ef4444', '#64748b'];

const BROWSE_PATHS = ['C:\\', 'C:\\Users', 'C:\\Windows\\Temp'];

export function AnalyzerView({ client, settings, onOpenLog, onOpenSettings }: { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void; onOpenSettings?: () => void }) {
  const [rootPath, setRootPath] = useState('C:\\');
  const [analyzedPath, setAnalyzedPath] = useState<string | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [progress, setProgress] = useState<AnalyzerProgressEvent | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [, setStartedAt] = useState<number | null>(null);
  const [result, setResult] = useState<StartAnalyzerResult | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [drawerExplain, setDrawerExplain] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<{ previewToken: string; breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[]; totalSelectedCount: string; totalSelectedBytes: string; issuedAtMs: number } | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'size' | 'items' | 'name'>('size');
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
    if (result?.root) walk(result.root);
    return map;
  }, [result]);

  const filteredRoot = useMemo(() => result ? filterTree(result.root, query, sort) : null, [result, query, sort]);
  const flatRows = useMemo(() => (filteredRoot ? buildFlat(filteredRoot, expanded) : []), [filteredRoot, expanded]);

  const selectedNodes = useMemo(() => Array.from(selectedIds).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[], [selectedIds, nodesById]);
  const selectedBytes = useMemo(() => selectedNodes.reduce((a, n) => a + BigInt(n.sizeBytes || '0'), 0n).toString(), [selectedNodes]);
  const requiresConfirmation = useMemo(() => analyzerSelectionNeedsConfirmation(selectedNodes), [selectedNodes]);

  function resetAnalyzerSession(msg?: string) {
    setSessionId(null); setStartedAt(null); setResult(null); setSelectedIds(new Set()); setExpanded(new Set());
    setPreview(null); setConfirmText(''); setCleanProgress(null); setInspected(null);
    if (msg) setInfo(msg);
  }

  async function inspectNode(itemId: string) {
    try { setInspected(await client.explainItem(itemId)); } catch { /* ignore */ }
  }

  async function runAnalyzer(pathOverride?: string) {
    const targetPath = (pathOverride ?? rootPath).trim() || 'C:\\';
    setRootPath(targetPath);
    setError(null); setInfo(null); setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds(new Set()); setResult(null); setExpanded(new Set()); setAnalyzing(true); setProgress(null); setInspected(null);
    try {
      const res = await client.startAnalyzer(targetPath, (p) => setProgress(p));
      setResult(res);
      setSessionId(res.sessionId);
      setStartedAt(Date.now());
      setExpanded(new Set([res.root.id]));
      setAnalyzedPath(res.rootPath || targetPath);
      try { setInspected(await client.explainItem(res.root.id)); } catch { /* ignore */ }
      setInfo(null);
    } catch (e) {
      setError(formatContractError(e));
    } finally { setAnalyzing(false); }
  }

  function toggleExpand(node: AnalyzerNodeDto) {
    if (!node.isDir || !node.children || node.children.length === 0) return;
    setExpanded((prev) => { const next = new Set(prev); if (next.has(node.id)) next.delete(node.id); else next.add(node.id); return next; });
  }

  function toggleSelect(node: AnalyzerNodeDto) {
    if (!isNodeSelectable(node, allowDeleteUnknown)) return;
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => { const next = new Set(prev); if (next.has(node.id)) next.delete(node.id); else next.add(node.id); return next; });
    void inspectNode(node.id);
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
    setError(null); setInfo(null); setPreviewLoading(true); setCleanSummary(null); setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(sessionId, Array.from(selectedIds), confirmations);
      setPreview({ ...res, issuedAtMs: Date.now() });
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
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
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Review & Clean again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !analyzing && !previewLoading && !cleaning && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !analyzing && !previewLoading && !cleaning;

  const allocSegments = useMemo(() => {
    type AllocSeg = { id: string; name: string; bytes: bigint; pct: number; pctVisual: number; color: string; isOther?: boolean };
    if (!result) return [] as AllocSeg[];
    const kids = [...(result.root.children ?? [])].sort((a, b) => {
      const sa = BigInt(a.sizeBytes || '0'), sb = BigInt(b.sizeBytes || '0');
      return sa === sb ? 0 : (sa > sb ? -1 : 1);
    });
    const total = BigInt(result.root.sizeBytes || '0') || 1n;
    const raw: AllocSeg[] = kids.slice(0, 5).map((k, i) => {
      const bytes = BigInt(k.sizeBytes || '0');
      const pct = Number((bytes * 10000n) / total) / 100;
      return { id: k.id, name: k.name, bytes, pct, pctVisual: pct, color: ALLOC_COLORS[i % ALLOC_COLORS.length] };
    });
    const rest = kids.slice(5);
    let otherBytes = 0n;
    for (const k of rest) otherBytes += BigInt(k.sizeBytes || '0');
    if (otherBytes > 0n) {
      const pct = Number((otherBytes * 10000n) / total) / 100;
      raw.push({ id: 'alloc-other', name: 'Other', bytes: otherBytes, pct, pctVisual: pct, color: '#64748b', isOther: true });
    }
    // enforce minimum visible width (~6px of 1000 viewbox) for any non-zero segment
    const minVisual = 0.6;
    let adjusted = raw.map((s) => ({ ...s, pctVisual: s.pct > 0 ? Math.max(minVisual, s.pct) : 0 }));
    const sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
    if (sum > 100) {
      // scale down proportionally while keeping min floor
      const over = sum - 100;
      const shrinkable = adjusted.filter((s) => s.pctVisual > minVisual);
      const shrinkSum = shrinkable.reduce((a, b) => a + (b.pctVisual - minVisual), 0);
      if (shrinkSum > 0) {
        adjusted = adjusted.map((s) => {
          if (s.pctVisual <= minVisual) return s;
          const room = s.pctVisual - minVisual;
          return { ...s, pctVisual: s.pctVisual - (over * room) / shrinkSum };
        });
      }
    } else if (sum < 100 && adjusted.length > 0) {
      const largest = [...adjusted].sort((a, b) => b.pct - a.pct)[0];
      adjusted = adjusted.map((s) => s.id === largest.id ? { ...s, pctVisual: s.pctVisual + (100 - sum) } : s);
    }
    return adjusted;
  }, [result]);

  function focusNode(nodeId: string) {
    const node = nodesById.get(nodeId);
    if (!node) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      const walk = (n: AnalyzerNodeDto, ancestors: string[]): boolean => {
        if (n.id === nodeId) { ancestors.forEach((a) => next.add(a)); return true; }
        if (!n.children) return false;
        for (const c of n.children) { if (walk(c, [...ancestors, n.id])) return true; }
        return false;
      };
      if (result) walk(result.root, []);
      return next;
    });
    void inspectNode(nodeId);
  }

  const allocRects = (() => {
    let x = 0;
    const vbW = 1000;
    const h = 14;
    const visible = allocSegments.filter((s) => s.pctVisual > 0);
    return allocSegments.map((s, idx) => {
      const w = Math.max(0, (s.pctVisual / 100) * vbW);
      const gap = visible.length > 1 && idx < allocSegments.length - 1 && w > 4 ? 2 : 0;
      const rect = (
        <rect
          key={s.id}
          x={x}
          y={0}
          width={Math.max(0, w - gap)}
          height={h}
          fill={s.color}
          className="alloc-segment"
          onClick={() => focusNode(s.id)}
        >
          <title>{`${s.name} · ${formatBytes(s.bytes.toString())} · ${s.pct < 0.1 ? '<0.1%' : `${s.pct.toFixed(1)}%`}`}</title>
        </rect>
      );
      x += w;
      return rect;
    });
  })();

  function cycleBrowsePath() {
    const idx = BROWSE_PATHS.findIndex((p) => p.toLowerCase() === rootPath.trim().toLowerCase());
    const next = BROWSE_PATHS[(idx + 1 + BROWSE_PATHS.length) % BROWSE_PATHS.length];
    setRootPath(next);
  }

  const inspectedNode = inspected ? nodesById.get(inspected.itemId) ?? null : null;
  const inspectedStatus = inspectedNode ? analyzerStatusText(inspectedNode, isNodeSelectable(inspectedNode, allowDeleteUnknown), allowDeleteUnknown) : null;
  const hasResult = !!result;

  const breadcrumbParts = useMemo(() => {
    const p = normalizeDisplayPath((analyzedPath || rootPath).trim() || 'C:\\');
    return p.split('\\').filter(Boolean).map((seg, i, arr) => ({ label: i === 0 ? `${seg}\\` : seg, path: arr.slice(0, i + 1).join('\\') + (i === 0 ? '\\' : '') }));
  }, [analyzedPath, rootPath]);

  return (
    <div className="view-stack">
      <header className="page-header" aria-label="Analyzer header">
        <div className="page-header-left">
          <h1 className="page-title" id="analyzer-heading">Analyzer</h1>
          {result && <span className="hero-metric-badge num" aria-label="Analyzed total">{formatBytes(result.totalBytes)} analyzed · {pluralize(Number(result.totalItems || '0'), 'item', 'items')}</span>}
        </div>
        <div className="page-header-right">
          <button
            key={hasResult ? 'reanalyze' : 'analyze'}
            className={hasResult ? 'btn secondary-action' : 'btn primary'}
            onClick={() => void runAnalyzer()}
            disabled={analyzing || cleaning || !rootPath.trim()}
            aria-label="Analyze"
          >
            {hasResult ? <><RefreshIcon aria-hidden="true" /> <span>Re-analyze</span></> : 'Analyze'}
          </button>
        </div>
      </header>

      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      <div className="analyzer-toolbar" aria-label="Analyzer controls">
        <label htmlFor="analyzer-root" className="muted small-note">Root</label>
        <div className="root-input-wrap">
          <input id="analyzer-root" type="text" value={rootPath} onChange={(e) => setRootPath(e.target.value)} aria-label="Analyzer root path" disabled={analyzing || cleaning} className="analyzer-root-input" />
          <button type="button" className="btn browse-btn" onClick={cycleBrowsePath} disabled={analyzing || cleaning} aria-label="Browse root paths"><BrowseIcon /> <span>Browse...</span></button>
        </div>
        <div className="quick-roots" aria-label="Quick root path presets">
          <span className="quick-roots-label muted">Quick roots:</span>
          {(['C:\\', 'C:\\Users', 'Temp'] as const).map((label) => {
            const value = label === 'Temp' ? 'C:\\Windows\\Temp' : label;
            return <button key={label} type="button" className="pill-btn" onClick={() => { setRootPath(value); if (hasResult) void runAnalyzer(value); }} disabled={analyzing || cleaning}>{label}</button>;
          })}
        </div>
        <input type="search" className="analyzer-search-input" placeholder="Search folders..." value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search analyzer folders" disabled={!result} />
        <label className="toolbar-group">Sort
          <select value={sort} onChange={(e) => setSort(e.target.value as 'size' | 'items' | 'name')} aria-label="Sort analyzer" disabled={!result}>
            <option value="size">Size</option>
            <option value="items">Items</option>
            <option value="name">Name</option>
          </select>
        </label>
      </div>
      {analyzedPath && (
        <div className="analyzer-breadcrumb" aria-label="Analyzed path">
          {breadcrumbParts.map((part, i) => (
            <span key={part.path} className="breadcrumb-seg">
              {i > 0 && <span className="breadcrumb-sep" aria-hidden="true">\</span>}
              <button type="button" className="breadcrumb-btn" onClick={() => { setRootPath(part.path); void runAnalyzer(part.path); }} disabled={analyzing || cleaning}>{part.label.replace(/\\$/, '')}</button>
            </span>
          ))}
        </div>
      )}

      {result && allocSegments.length > 0 && (
        <section className="alloc-bar-wrap" aria-label="Analyzer allocation">
          <svg className="alloc-bar" viewBox="0 0 1000 14" preserveAspectRatio="none" role="img" aria-label="Top-level folder allocation">
            {allocRects}
          </svg>
          <div className="alloc-legend">
            {allocSegments.map((s) => (
              <button key={s.id} type="button" className="alloc-legend-item alloc-legend-btn" onClick={() => focusNode(s.id)} title={`Inspect ${s.name}`}>
                <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><rect x="0" y="0" width="8" height="8" fill={s.color} rx="2" /></svg>
                <span>{s.name}</span>
                <span className="muted num">{formatBytes(s.bytes.toString())} · {s.pct < 0.1 ? '<0.1%' : `${s.pct.toFixed(1)}%`}</span>
              </button>
            ))}
          </div>
        </section>
      )}

      {analyzing && (
        <div role="status" aria-live="polite" className="notice info scanning-pulse">
          Analyzing… {progress ? `dirs ${progress.scannedDirs}, files ${progress.scannedFiles}, ${formatBytes(progress.scannedBytes)}${progress.currentPath ? ` — ${normalizeDisplayPath(progress.currentPath)}` : ''}` : 'Starting…'}
          <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
        </div>
      )}

      {result && (
        <>
          <section className="master-detail analyzer-layout scan-layout-with-bar" aria-label="Analyzer results">
            <div className="master-pane">
              <div className="pane-header">
                <div className="pane-header-left">
                  <h2 id="analyzer-tree-heading">Analyzer Tree</h2>
                  <span className="pane-meta-pill num">{formatBytes(result.totalBytes)} · {pluralize(Number(result.totalItems || '0'), 'item', 'items')}</span>
                </div>
              </div>
              <div className="pane-body flush">
                <div
                  className="analyzer-viewport"
                  role="tree"
                  aria-label="Analyzer results tree"
                  tabIndex={0}
                >
                  {flatRows.map(({ node, depth, siblingMax }) => {
                    const selectable = isNodeSelectable(node, allowDeleteUnknown);
                    const checked = selectedIds.has(node.id);
                    const hasChildren = node.isDir && (node.children?.length ?? 0) > 0;
                    const isExpanded = expanded.has(node.id);
                    const status = analyzerStatusText(node, selectable, allowDeleteUnknown);
                    const badge = guardBadge(node);
                    const typeTag = nodeTypeTag(node);
                    const nodeSize = BigInt(node.sizeBytes || '0');
                    const share = siblingMax > 0n ? Number((nodeSize * 10000n) / siblingMax) / 100 : 0;
                    const shareLabel = share < 0.1 ? '<0.1%' : `${share.toFixed(1)}%`;
                    const pathDisplay = normalizeDisplayPath(node.path);
                    const inspectedMatch = inspected?.itemId === node.id;
                    return (
                      <div
                        key={node.id}
                        role="treeitem"
                        aria-expanded={hasChildren ? isExpanded : undefined}
                        aria-selected={checked}
                        aria-level={depth + 1}
                        tabIndex={0}
                        className={`analyzer-row ${checked ? 'selected' : ''} ${inspectedMatch ? 'inspected' : ''}`}
                        onClick={() => { void inspectNode(node.id); if (selectable) toggleSelect(node); }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault();
                            void inspectNode(node.id);
                            if (selectable) toggleSelect(node);
                          }
                        }}
                      >
                        <div className="analyzer-col-main">
                          <span className="tree-indent" aria-hidden="true">
                            {Array.from({ length: depth }).map((_, i) => <span key={i} className="indent-step" />)}
                          </span>
                          {hasChildren ? (
                            <button
                              type="button"
                              className="tree-toggle"
                              onClick={(e) => { e.stopPropagation(); toggleExpand(node); }}
                              aria-label={isExpanded ? `Collapse ${node.name}` : `Expand ${node.name}`}
                            >
                              {isExpanded ? <ChevronDownIcon /> : <ChevronRightIcon />}
                            </button>
                          ) : (
                            <span className="tree-toggle-spacer" aria-hidden="true" />
                          )}
                          <span className={`analyzer-select-wrap ${status.locked ? 'locked' : ''}`} onClick={(e) => e.stopPropagation()}>
                            {status.locked ? (
                              <label className="checkbox analyzer-checkbox analyzer-checkbox-locked" title="Not selectable">
                                <LockIcon className="analyzer-lock-only" aria-hidden="true" />
                                <input
                                  type="checkbox"
                                  aria-label={`Select analyzer node ${node.path}`}
                                  checked={checked}
                                  disabled
                                  readOnly
                                />
                              </label>
                            ) : (
                              <label className="checkbox analyzer-checkbox" title={selectable ? 'Select for Review & Clean' : 'Not selectable'}>
                                <input
                                  type="checkbox"
                                  aria-label={`Select analyzer node ${node.path}`}
                                  checked={checked}
                                  disabled={!selectable || cleaning}
                                  onChange={() => toggleSelect(node)}
                                />
                              </label>
                            )}
                          </span>
                          <div className="analyzer-node-main">
                            <div className="analyzer-name-line">
                              <span className="analyzer-name" title={node.name}>{node.name}</span>
                              {typeTag && <span className="inline-type-tag muted">· {typeTag}</span>}
                            </div>
                            <div className="analyzer-path" title={pathDisplay}>{pathDisplay}</div>
                            <span className="sr-only">{status.text}</span>
                          </div>
                        </div>
                        <div className="analyzer-col-share">
                          <progress className="analyzer-size-bar" value={Math.min(100, Math.max(0, share))} max={100} aria-label={`Share ${shareLabel}`} />
                          <span className="analyzer-share-num num">{shareLabel}</span>
                          <span className="item-size num">{formatBytes(node.sizeBytes)}</span>
                        </div>
                        <div className="analyzer-col-count muted num">{node.itemCount}</div>
                        <div className="analyzer-col-guard">
                          {badge ? <span className={badge === 'Read-Only' ? 'status-muted status-readonly' : (badge === 'Protected' || badge === 'Unknown' || badge === 'Caution' ? riskClass(badge as RiskDto) : 'status-muted')}>{badge}</span> : <span className="status-muted-inline" />}
                        </div>
                      </div>
                    );
                  })}
                  {flatRows.length === 0 && <div className="muted analyzer-empty">No nodes.</div>}
                </div>
              </div>
            </div>

            <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="analyzer-preview-heading">
              <div className="pane-header">
                <h3 id="analyzer-preview-heading">Inspector</h3>
              </div>
              <div className="detail-scroll">
                <div className="inspector-section" aria-label="Analyzer inspector">
                  {inspected ? (
                    <>
                      <p className="inspector-headline">
                        <span className="inspector-headline-name" title={normalizeDisplayPath(inspectedNode?.path ?? inspected.itemId)}>{inspected.title}</span>
                        <span className={riskClass(inspected.risk)}>{inspected.risk}</span>
                      </p>
                      <dl className="kv kv-stacked">
                        <div className="kv-block"><dt>What is it?</dt><dd>{inspected.description}</dd></div>
                        <div className="kv-block"><dt>What happens if deleted?</dt><dd>{inspected.risk === 'Protected' ? 'Nothing will be deleted. Protected Paths are never removable.' : (inspectedNode && (inspectedNode.displayOnly || inspectedNode.isLinkPoint || inspectedNode.isCloudPlaceholder || inspectedNode.path.toLowerCase().endsWith('.vhdx')) ? 'This item is display-only and cannot be deleted from Analyzer.' : 'The selected item is permanently deleted and bypasses the Recycle Bin when you confirm Review & Clean.')}</dd></div>
                        <div className="kv-block"><dt>Status &amp; Eligibility</dt><dd>{inspectedStatus ? inspectedStatus.text : '—'}</dd></div>
                        {inspectedNode?.tier && <div className="kv-block"><dt>Tier</dt><dd>{tierLabel(inspectedNode.tier) ?? inspectedNode.tier}</dd></div>}
                        <div className="kv-block kv-path-block"><dt>Full Path</dt>
                          <dd>
                            <div className="path-well">
                              <span className="mono path-well-text" title={normalizeDisplayPath(inspectedNode?.path ?? inspected.itemId)}>{normalizeDisplayPath(inspectedNode?.path ?? inspected.itemId)}</span>
                              <button type="button" className="icon-only-btn subtle path-copy-btn" onClick={() => void copyText(normalizeDisplayPath(inspectedNode?.path ?? inspected.itemId))} aria-label="Copy path" title="Copy path"><CopyIcon /></button>
                            </div>
                          </dd>
                        </div>
                      </dl>
                      <div className="row inspector-actions">
                        <button className="btn small" onClick={() => void onExplain(inspected.itemId)} aria-label="Explain">Explain</button>
                        {inspectedStatus?.unknownGated && onOpenSettings && (
                          <button className="btn small" onClick={onOpenSettings}>Open Settings →</button>
                        )}
                      </div>
                    </>
                  ) : (
                    <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>
                  )}
                </div>

                {null}

                {requiresConfirmation && (
                  <div className="notice warn" role="note">
                    <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
                    <div className="field">
                      <label htmlFor="analyzer-confirm-delete">Typed confirmation</label>
                      <input id="analyzer-confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={cleaning} />
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
                  <div className="clean-summary-section" aria-labelledby="analyzer-clean-summary-heading" role="status">
                    <h4 id="analyzer-clean-summary-heading" className="clean-summary-title">Clean Summary</h4>
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
          <div className="scan-bottom-bar" aria-label="Analyzer actions">
            <div className="scan-bottom-bar-left">
              <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(selectedIds.size, 'item', 'items')} selected</strong> · <span className="num">{formatBytes(selectedBytes)}</span></span>
              {selectedIds.size > 0 && <button type="button" className="btn small subtle-btn" onClick={() => { setSelectedIds(new Set()); setPreview(null); }} disabled={cleaning}>Clear</button>}
            </div>
            <div className="scan-bottom-bar-right">
              {!preview ? (
                <button className={`btn ${selectedIds.size > 0 ? 'primary' : ''}`} onClick={runPreview} disabled={!canPreview} aria-label="Review & Clean">{previewLoading ? 'Generating Preview…' : `Review & Clean${selectedIds.size > 0 ? ` (${formatBytes(selectedBytes)})` : ''}`}</button>
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
