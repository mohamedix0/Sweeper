import { useMemo, useRef, useState } from 'react';
import {
  formatContractError,
  type AnalyzerProgressEvent,
  type CleanProgressEvent,
  type ExecuteCleanResponse,
  type Settings,
  type StartAnalyzerResult,
  type SweeperClient,
} from '../api/client';
import type { AnalyzerNodeDto, ContractError } from '../contract/bindings';
import { formatBytes } from '../utils/format';
import {
  ExplainDrawer,
  PREVIEW_TTL_MS,
  SESSION_TTL_MS,
  TtlIndicator,
  TYPED_CONFIRMATION_PHRASE,
  riskClass,
  type ExplainState,
  type PreviewResult,
} from './cleanShared';
import { ChevronDownIcon, ChevronRightIcon, DotIcon } from './Icons';

type FlatAnalyzerRow = { node: AnalyzerNodeDto; depth: number };

function flattenVisible(root: AnalyzerNodeDto, expanded: Set<string>): FlatAnalyzerRow[] {
  const rows: FlatAnalyzerRow[] = [];
  const walk = (node: AnalyzerNodeDto, depth: number) => {
    rows.push({ node, depth });
    if (node.isDir && expanded.has(node.id) && node.children && node.children.length > 0) {
      for (const c of node.children) walk(c, depth + 1);
    }
  };
  walk(root, 0);
  return rows;
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

export function AnalyzerView({ client, settings, onOpenLog }: { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void }) {
  const [rootPath, setRootPath] = useState('C:\\');
  const [analyzing, setAnalyzing] = useState(false);
  const [progress, setProgress] = useState<AnalyzerProgressEvent | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [result, setResult] = useState<StartAnalyzerResult | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [explain, setExplain] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
    if (result?.root) walk(result.root);
    return map;
  }, [result]);

  const flatRows = useMemo(() => (result ? flattenVisible(result.root, expanded) : []), [result, expanded]);
  const rowHeight = 54;
  const viewportHeight = 432;
  const overscan = 8;
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
  const endIndex = Math.min(flatRows.length, startIndex + visibleCount);
  const visibleRows = flatRows.slice(startIndex, endIndex);
  const topPad = startIndex * rowHeight;
  const bottomPad = Math.max(0, (flatRows.length - endIndex) * rowHeight);

  const selectedNodes = useMemo(() => Array.from(selectedIds).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[], [selectedIds, nodesById]);
  const selectedBytes = useMemo(() => selectedNodes.reduce((a, n) => a + BigInt(n.sizeBytes), 0n).toString(), [selectedNodes]);
  const requiresConfirmation = useMemo(() => analyzerSelectionNeedsConfirmation(selectedNodes), [selectedNodes]);

  function resetAnalyzerSession(msg?: string) {
    setSessionId(null); setStartedAt(null); setResult(null); setSelectedIds(new Set()); setExpanded(new Set());
    setPreview(null); setConfirmText(''); setCleanProgress(null);
    if (msg) setInfo(msg);
  }

  async function runAnalyzer() {
    setError(null); setInfo(null); setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds(new Set()); setResult(null); setExpanded(new Set()); setAnalyzing(true); setProgress(null);
    try {
      const res = await client.startAnalyzer(rootPath || null, (p) => setProgress(p));
      setResult(res);
      setSessionId(res.sessionId);
      setStartedAt(Date.now());
      setExpanded(new Set([res.root.id]));
      setInfo('Analyzer complete. Review the tree, select eligible nodes, then Preview Selected to continue through the standard Clean flow.');
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
  }

  async function onExplain(itemId: string) {
    setError(null);
    try { setExplain(await client.explainItem(itemId)); } catch (e) { setError(formatContractError(e)); }
  }

  async function runPreview() {
    if (!sessionId) return;
    setError(null); setInfo(null); setPreviewLoading(true); setCleanSummary(null); setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(sessionId, Array.from(selectedIds), confirmations);
      setPreview({ ...res, issuedAtMs: Date.now() });
      setInfo('Preview ready. Review the breakdown and the permanent deletion warning before cleaning.');
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
      setInfo('Clean completed. Review the summary and operation log below.');
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Preview Selected again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !analyzing && !previewLoading && !cleaning && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !analyzing && !previewLoading && !cleaning;

  return (
    <div>
      <section className="panel" aria-labelledby="analyzer-heading">
        <h2 id="analyzer-heading">Analyzer</h2>
        <p className="muted">Analyze a folder to build an expandable tree of space usage. Select eligible nodes and route them through the standard Preview &rarr; Clean flow (no direct delete).</p>
        <div className="analyzer-toolbar">
          <label htmlFor="analyzer-root" className="muted">Root path</label>
          <input id="analyzer-root" type="text" value={rootPath} onChange={(e) => setRootPath(e.target.value)} aria-label="Analyzer root path" disabled={analyzing || cleaning} className="analyzer-root-input" />
          <button className="btn primary" onClick={runAnalyzer} disabled={analyzing || cleaning || !rootPath.trim()}>Analyze</button>
          {sessionId && startedAt && (
            <div className="meta-strip" aria-label="Analyzer session status">
              <span className="meta-chip num">Session: {sessionId}</span>
              <TtlIndicator label="Analyzer session TTL (15m)" issuedAtMs={startedAt} ttlMs={SESSION_TTL_MS} />
            </div>
          )}
        </div>
        {analyzing && (
          <div role="status" aria-live="polite" className="notice info">
            Analyzing… {progress ? `dirs ${progress.scannedDirs}, files ${progress.scannedFiles}, ${formatBytes(progress.scannedBytes)}${progress.currentPath ? ` — ${progress.currentPath}` : ''}` : 'Starting…'}
            <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
          </div>
        )}
        {null}
      </section>

      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      {result && (
        <section className="panel" aria-labelledby="analyzer-tree-heading">
          <div className="row">
            <h3 id="analyzer-tree-heading">Analyzer Tree</h3>
            <div className="spacer" />
            <span className="muted num">Total: {formatBytes(result.totalBytes)} ({result.totalItems} items)</span>
            <span className="muted num">Selected: {selectedIds.size} ({formatBytes(selectedBytes)})</span>
          </div>
          <div
            ref={viewportRef}
            className="analyzer-viewport"
            role="tree"
            aria-label="Analyzer results tree"
            tabIndex={0}
            onScroll={(e) => setScrollTop((e.target as HTMLDivElement).scrollTop)}
          >
            <div style={{ height: topPad }} />
            {visibleRows.map(({ node, depth }) => {
              const selectable = isNodeSelectable(node, allowDeleteUnknown);
              const checked = selectedIds.has(node.id);
              const hasChildren = node.isDir && (node.children?.length ?? 0) > 0;
              const isExpanded = expanded.has(node.id);
              const reasons: string[] = [];
              if (node.risk === 'Protected') reasons.push('Protected');
              if (node.risk === 'Unknown' && !allowDeleteUnknown) reasons.push('Unknown gated by Settings (Allow deleting unknown folders)');
              if (node.isLinkPoint) reasons.push('Link Point');
              if (node.isCloudPlaceholder) reasons.push('Cloud placeholder (Skipped)');
              if (node.displayOnly) reasons.push('Display-only');
              if (node.path.toLowerCase().endsWith('.vhdx')) reasons.push('.vhdx display-only');
              if (node.reason) reasons.push(node.reason);
              return (
                  <div
                    key={node.id}
                    role="treeitem"
                    aria-expanded={hasChildren ? isExpanded : undefined}
                    aria-selected={checked}
                    aria-level={depth + 1}
                    className="analyzer-row"
                  >
                    <div className="analyzer-tree-cell" style={{ paddingLeft: 8 + depth * 18 }}>
                      <button
                        type="button"
                        className="tree-toggle"
                        onClick={() => toggleExpand(node)}
                        aria-label={hasChildren ? (isExpanded ? `Collapse ${node.name}` : `Expand ${node.name}`) : `Leaf ${node.name}`}
                        disabled={!hasChildren}
                      >
                        {hasChildren ? (isExpanded ? <ChevronDownIcon /> : <ChevronRightIcon />) : <DotIcon />}
                      </button>
                      <label className="checkbox" title={selectable ? "Select for Preview/Clean" : "Not selectable"}>
                        <input
                          type="checkbox"
                          aria-label={`Select analyzer node ${node.path}`}
                          checked={checked}
                          disabled={!selectable || cleaning}
                          onChange={() => toggleSelect(node)}
                        />
                      </label>
                      <div className="analyzer-node-main">
                        <div className="item-path">{node.name} <span className="muted">({node.path})</span></div>
                        <div className="item-status">
                          {selectable ? (node.risk === "Unknown" || !node.targetId ? "Selectable (typed confirmation required at Preview)" : "Selectable") : `Not selectable${reasons.length ? `: ${reasons.join(", ")}` : ""}`}
                        </div>
                      </div>
                    </div>
                    <div className="item-size num">{formatBytes(node.sizeBytes)}</div>
                    <div className="muted num">{node.itemCount}</div>
                    <button className="btn" onClick={() => onExplain(node.id)}>Explain</button>
                    <div className="analyzer-badges">
                      <span className={riskClass(node.risk)} aria-label={`Risk ${node.risk}`}>{node.risk}</span>
                      {node.tier && <span className="badge">{node.tier}</span>}
                      {node.isLinkPoint && <span className="badge">Link Point</span>}
                      {node.displayOnly && <span className="badge">Display-only</span>}
                    </div>
                  </div>
                );
            })}
            <div style={{ height: bottomPad }} />
            {flatRows.length === 0 && <div className="muted analyzer-empty">No nodes.</div>}
          </div>

          <section aria-labelledby="analyzer-preview-heading" className="panel panel-spaced">
            <h3 id="analyzer-preview-heading">Preview Selected</h3>
            <p className="muted">Selection total: <strong className="num">{formatBytes(selectedBytes)}</strong> <span className="num">({selectedIds.size} node(s))</span>. Analyzer selections go through the standard Preview &rarr; Clean flow only.</p>
            {requiresConfirmation && (
              <div className="notice warn" role="note">
                <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
                <div className="field">
                  <label htmlFor="analyzer-confirm-delete">Typed confirmation</label>
                  <input id="analyzer-confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={cleaning} />
                </div>
              </div>
            )}
            <div className="row"><button className="btn primary" onClick={runPreview} disabled={!canPreview}>{previewLoading ? 'Generating Preview…' : 'Preview Selected'}</button></div>
            {preview && (
              <div aria-live="polite" className="preview-breakdown">
                <h4 className="section-title preview-breakdown-title">Preview Breakdown</h4>
                <div className="meta-strip preview-meta-strip">
                  <span className="meta-chip meta-chip-code">Preview token: <code>{preview.previewToken}</code></span>
                  <TtlIndicator label="Preview token TTL (10m)" issuedAtMs={preview.issuedAtMs} ttlMs={PREVIEW_TTL_MS} />
                  {startedAt && <TtlIndicator label="Analyzer session TTL (15m)" issuedAtMs={startedAt} ttlMs={SESSION_TTL_MS} />}
                </div>
                <p className="preview-total"><strong>Total selected:</strong> {preview.totalSelectedCount} item(s), {formatBytes(preview.totalSelectedBytes)}</p>
                <table className="table" aria-label="Preview breakdown by Target">
                  <thead><tr><th>Target</th><th>Selected count</th><th>Selected bytes</th></tr></thead>
                  <tbody>
                    {preview.breakdown.map((b) => (
                      <tr key={b.targetId}><td>{b.targetName} <span className="muted">({b.targetId})</span></td><td>{b.selectedCount}</td><td>{formatBytes(b.selectedBytes)}</td></tr>
                    ))}
                  </tbody>
                </table>
                <div className="clean-footer" role="alert" aria-live="polite">
                  <div className="clean-footer-warning"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div>
                  <button className="btn danger primary" onClick={runClean} disabled={!canClean} aria-label="Clean Permanently">{cleaning ? 'Cleaning…' : 'Clean Permanently'}</button>
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
              <div className="panel" aria-labelledby="analyzer-clean-summary-heading" role="status">
                <h4 id="analyzer-clean-summary-heading" className="section-title">Clean Summary</h4>
                {cleanSummary.aborted && <div className="notice error" role="alert"><strong>Clean aborted.</strong> {cleanSummary.abortReason || 'The operation was aborted.'}</div>}
                <dl className="kv">
                  <dt>Bytes freed</dt><dd>{formatBytes(cleanSummary.bytesFreed)}</dd>
                  <dt>Deleted count</dt><dd>{cleanSummary.deletedCount}</dd>
                  <dt>Skipped count</dt><dd>{cleanSummary.skippedCount}</dd>
                </dl>
                <div>
                  <strong>Skipped reasons</strong>
                  {cleanSummary.skippedReasons.length === 0 ? <p className="muted" aria-label="Skipped reasons">None.</p> : (
                    <ul aria-label="Skipped reasons">{cleanSummary.skippedReasons.map((r, idx) => <li key={`${r.reason}-${idx}`}>{r.reason} — {r.count}</li>)}</ul>
                  )}
                </div>
                {cleanSummary.logId && <div className="row row-top"><button className="btn" onClick={() => onOpenLog(cleanSummary.logId!)}>View Operation Log</button><span className="muted">Log: {cleanSummary.logId}</span></div>}
              </div>
            )}
          </section>
        </section>
      )}
      <ExplainDrawer explain={explain} onClose={() => setExplain(null)} />
    </div>
  );
}
