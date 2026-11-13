import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import './App.css';
import {
  createClient,
  formatContractError,
  type AnalyzerProgressEvent,
  type CleanProgressEvent,
  type ExecuteCleanResponse,
  type ScanResults,
  type StartAnalyzerResult,
  type SweeperClient,
} from './api/client';
import type { AnalyzerNodeDto, ContractError, RiskDto, TargetScanResultDto } from './contract/bindings';
import { formatBytes, formatUnixSeconds, sumDecimalStrings } from './utils/format';

type View = 'scan' | 'analyzer' | 'settings' | 'logs';

type ExplainState = { itemId: string; title: string; description: string; risk: RiskDto } | null;
type PreviewResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  issuedAtMs: number;
};

const SESSION_TTL_MS = 15 * 60 * 1000;
const PREVIEW_TTL_MS = 10 * 60 * 1000;

function riskClass(risk: RiskDto): string {
  return `badge risk-${risk}`;
}

function isSelectableItem(item: { ageEligible: boolean }, targetRisk: RiskDto): boolean {
  if (targetRisk === 'Protected') return false;
  if (!item.ageEligible) return false;
  return true;
}

function needsTypedConfirmation(targets: TargetScanResultDto[], selectedIds: Set<string>): boolean {
  for (const t of targets) {
    if (t.risk === 'Unknown') {
      if (t.items.some((i) => selectedIds.has(i.id))) return true;
    }
    for (const item of t.items) {
      if (!selectedIds.has(item.id)) continue;
      if (t.targetId.startsWith('custom-')) return true;
    }
  }
  return false;
}

function TtlIndicator({ label, issuedAtMs, ttlMs }: { label: string; issuedAtMs: number; ttlMs: number }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, []);
  const remaining = Math.max(0, ttlMs - (Date.now() - issuedAtMs));
  const mins = Math.floor(remaining / 60000);
  const secs = Math.floor((remaining % 60000) / 1000);
  return (
    <span className="muted">
      {label}: {mins}:{secs.toString().padStart(2, '0')} remaining
    </span>
  );
}

function ExplainDrawer({ explain, onClose }: { explain: ExplainState; onClose: () => void }) {
  if (!explain) return null;
  return (
    <div className="drawer-backdrop" role="presentation" onClick={onClose}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="explain-title"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
        }}
      >
        <div className="row">
          <h3 id="explain-title">Explain: {explain.title}</h3>
          <div className="spacer" />
          <button className="btn" onClick={onClose} aria-label="Close explanation">Close</button>
        </div>
        <p><span className={riskClass(explain.risk)}>Risk: {explain.risk}</span></p>
        <p>{explain.description}</p>
        <dl className="kv">
          <dt>Item ID</dt><dd>{explain.itemId}</dd>
        </dl>
      </aside>
    </div>
  );
}

function ScanAndPreviewView({ client, onOpenLog }: { client: SweeperClient; onOpenLog: (logId: string) => void }) {
  const [tier1, setTier1] = useState(true);
  const [tier2, setTier2] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanStatus, setScanStatus] = useState<string>('');
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [scanStartedAt, setScanStartedAt] = useState<number | null>(null);
  const [results, setResults] = useState<ScanResults | null>(null);
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

  const targets = useMemo(() => results?.targets ?? [], [results]);

  const selectedBytes = useMemo(() => {
    const sizes: string[] = [];
    for (const t of targets) for (const item of t.items) if (selectedIds.has(item.id)) sizes.push(item.size);
    return sumDecimalStrings(sizes);
  }, [targets, selectedIds]);

  const selectableCount = useMemo(() => {
    let n = 0;
    for (const t of targets) for (const i of t.items) if (isSelectableItem(i, t.risk)) n++;
    return n;
  }, [targets]);

  const requiresConfirmation = useMemo(() => needsTypedConfirmation(targets, selectedIds), [targets, selectedIds]);

  function resetForInvalidSession(message?: string) {
    setSessionId(null);
    setScanStartedAt(null);
    setResults(null);
    setSelectedIds(new Set());
    setPreview(null);
    setConfirmText('');
    setCleanProgress(null);
    if (message) setInfo(message);
  }

  async function runScan() {
    setError(null); setInfo(null); setPreview(null); setExplain(null); setSelectedIds(new Set());
    setResults(null); setCleanSummary(null); setCleanProgress(null); setScanning(true);
    setScanStatus('Starting Scan…');
    const tiers = [tier1 ? 'Tier1' : null, tier2 ? 'Tier2' : null].filter(Boolean) as ('Tier1' | 'Tier2')[];
    try {
      const { sessionId: sid } = await client.startScan(tiers.length ? tiers : null, (ev) => {
        if (ev.kind === 'started') setScanStatus('Scan in progress…');
        if (ev.kind === 'completed') setScanStatus('Scan completed.');
        if (ev.kind === 'failed') setScanStatus('Scan failed.');
      });
      setSessionId(sid);
      setScanStartedAt(Date.now());
      const data = await client.getScanResults(sid);
      setResults(data);
      setInfo('Scan complete. Select eligible items, run Preview, then Clean Permanently if you confirm.');
    } catch (e) {
      setError(formatContractError(e));
      const ce = e as ContractError | undefined;
      if (ce?.code === 'sessionInvalid') resetForInvalidSession();
    } finally {
      setScanning(false);
    }
  }

  function toggleItem(itemId: string, targetRisk: RiskDto, ageEligible: boolean) {
    if (!isSelectableItem({ ageEligible }, targetRisk)) return;
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => { const next = new Set(prev); if (next.has(itemId)) next.delete(itemId); else next.add(itemId); return next; });
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
      if (ce?.code === 'sessionInvalid') resetForInvalidSession('ScanSession expired or was invalidated. Start a new Scan.');
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
      setInfo('Clean completed. Review the summary and operation log below.');
      if (sessionId) { try { setResults(await client.getScanResults(sessionId)); } catch { /* ignore */ } }
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetForInvalidSession('ScanSession expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Preview again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading && !cleaning && (!requiresConfirmation || confirmText.trim().length > 0);
  const canClean = !!preview && !scanning && !previewLoading && !cleaning;

  return (
    <div>
      <section className="panel" aria-labelledby="scan-heading">
        <h2 id="scan-heading">Scan &amp; Preview</h2>
        <p className="muted">Discover reclaimable Targets, explain items, preview selections, and run permanent Clean only after confirmation.</p>
        <div className="row">
          <label className="checkbox"><input type="checkbox" checked={tier1} onChange={(e) => setTier1(e.target.checked)} /> Tier 1 (system temp and caches)</label>
          <label className="checkbox"><input type="checkbox" checked={tier2} onChange={(e) => setTier2(e.target.checked)} /> Tier 2 (developer caches)</label>
          <div className="spacer" />
          <button className="btn primary" onClick={runScan} disabled={scanning || cleaning}>Start Scan</button>
        </div>
        {scanning && (
          <div role="status" aria-live="polite" className="notice info">
            {scanStatus || 'Scanning…'}
            <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
          </div>
        )}
        {!scanning && scanStatus && <div className="notice">{scanStatus}</div>}
        {sessionId && scanStartedAt && (
          <div className="row muted" aria-label="Scan session status">
            <span>Session: {sessionId}</span>
            <TtlIndicator label="ScanSession TTL (15m)" issuedAtMs={scanStartedAt} ttlMs={SESSION_TTL_MS} />
          </div>
        )}
      </section>

      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      {results && (
        <section className="panel" aria-labelledby="results-heading">
          <div className="row">
            <h2 id="results-heading">Scan Results</h2>
            <div className="spacer" />
            <span className="muted">Selectable eligible items: {selectableCount}</span>
            <span className="muted">Selected: {selectedIds.size} ({formatBytes(selectedBytes)})</span>
          </div>
          {targets.length === 0 && <p className="muted">No Targets returned for this Scan.</p>}
          {targets.map((t) => (
            <article key={t.targetId} className="target-card" aria-labelledby={`target-${t.targetId}-name`}>
              <div className="target-header">
                <div className="target-meta">
                  <div className="row">
                    <strong id={`target-${t.targetId}-name`}>{t.targetName}</strong>
                    <span className={riskClass(t.risk)}>{t.risk}</span>
                    <span className="badge">{t.tier}</span>
                  </div>
                  <div className="target-stats">
                    <span>Size: {formatBytes(t.totalBytes)}</span>
                    <span>Eligible: {formatBytes(t.eligibleBytes)}</span>
                    <span>Files: {t.fileCount}</span>
                    <span>Eligible count: {t.eligibleCount}</span>
                    <span>Skipped: {t.skippedCount}</span>
                  </div>
                  <div className="muted">Resolved path(s): {t.resolvedPaths.join(', ') || '—'}</div>
                  {t.statusReason && <div className="muted">{t.statusReason}</div>}
                </div>
              </div>
              <div className="items" role="group" aria-label={`${t.targetName} items`}>
                {t.items.length === 0 && <div className="item-row"><span className="muted">No items.</span></div>}
                {t.items.map((item) => {
                  const selectable = isSelectableItem(item, t.risk);
                  const checked = selectedIds.has(item.id);
                  const reason = !item.ageEligible ? 'Not eligible (Minimum Age / recent or ineligible)' : (t.risk === 'Protected' ? 'Protected — never selectable' : 'Eligible');
                  return (
                    <div key={item.id} className="item-row">
                      <label className="checkbox">
                        <input type="checkbox" aria-label={`Select ${item.path}`} checked={checked} disabled={!selectable || cleaning} onChange={() => toggleItem(item.id, t.risk, item.ageEligible)} />
                      </label>
                      <div>
                        <div className="item-path">{item.path}</div>
                        <div className="item-status">{!selectable ? `Skipped/Not selectable: ${reason}` : 'Eligible for Preview selection'}</div>
                      </div>
                      <div className="item-size">{formatBytes(item.size)}</div>
                      <button className="btn" onClick={() => onExplain(item.id)}>Explain</button>
                      <span className={riskClass(t.risk)} aria-label={`Risk ${t.risk}`}>{t.risk}</span>
                    </div>
                  );
                })}
              </div>
            </article>
          ))}

          <section aria-labelledby="preview-heading" className="panel">
            <h3 id="preview-heading">Preview</h3>
            <p className="muted">Selection total: <strong>{formatBytes(selectedBytes)}</strong> ({selectedIds.size} item(s)). Preview requires a valid ScanSession and issues a single-use preview token (10m, bounded by session).</p>
            {requiresConfirmation && (
              <div className="notice warn" role="note">
                <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path items). Enter <code>DELETE</code> to request Preview.</div>
                <div className="field">
                  <label htmlFor="confirm-delete">Typed confirmation</label>
                  <input id="confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder="Type DELETE" autoComplete="off" disabled={cleaning} />
                </div>
              </div>
            )}
            <div className="row"><button className="btn primary" onClick={runPreview} disabled={!canPreview}>{previewLoading ? 'Generating Preview…' : 'Preview Clean'}</button></div>
            {preview && (
              <div aria-live="polite">
                <h4 className="section-title">Preview Breakdown</h4>
                <div className="row">
                  <span>Preview token: <code>{preview.previewToken}</code></span>
                  <TtlIndicator label="Preview token TTL (10m)" issuedAtMs={preview.issuedAtMs} ttlMs={PREVIEW_TTL_MS} />
                  {scanStartedAt && <TtlIndicator label="Session TTL (15m)" issuedAtMs={scanStartedAt} ttlMs={SESSION_TTL_MS} />}
                </div>
                <p><strong>Total selected:</strong> {preview.totalSelectedCount} item(s), {formatBytes(preview.totalSelectedBytes)}</p>
                <table className="table" aria-label="Preview breakdown by Target">
                  <thead><tr><th>Target</th><th>Selected count</th><th>Selected bytes</th></tr></thead>
                  <tbody>
                    {preview.breakdown.map((b) => (
                      <tr key={b.targetId}><td>{b.targetName} <span className="muted">({b.targetId})</span></td><td>{b.selectedCount}</td><td>{formatBytes(b.selectedBytes)}</td></tr>
                    ))}
                  </tbody>
                </table>
                <div className="notice warn" role="alert" aria-live="polite"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div>
                <div className="row"><button className="btn danger primary" onClick={runClean} disabled={!canClean} aria-label="Clean Permanently">{cleaning ? 'Cleaning…' : 'Clean Permanently'}</button></div>
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
              <div className="panel" aria-labelledby="clean-summary-heading" role="status">
                <h4 id="clean-summary-heading" className="section-title">Clean Summary</h4>
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
                {cleanSummary.logId && <div className="row" style={{ marginTop: '0.5rem' }}><button className="btn" onClick={() => onOpenLog(cleanSummary.logId!)}>View Operation Log</button><span className="muted">Log: {cleanSummary.logId}</span></div>}
              </div>
            )}
          </section>
        </section>
      )}
      <ExplainDrawer explain={explain} onClose={() => setExplain(null)} />
    </div>
  );
}

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

function isNodeSelectable(node: AnalyzerNodeDto): boolean {
  return !!node.selectable && !node.displayOnly && !node.isLinkPoint && !node.isCloudPlaceholder && node.risk !== 'Protected';
}

function AnalyzerView({ client, onOpenLog }: { client: SweeperClient; onOpenLog: (logId: string) => void }) {
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

  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
    if (result?.root) walk(result.root);
    return map;
  }, [result]);

  const flatRows = useMemo(() => (result ? flattenVisible(result.root, expanded) : []), [result, expanded]);
  const rowHeight = 34;
  const viewportHeight = 420;
  const overscan = 8;
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const visibleCount = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
  const endIndex = Math.min(flatRows.length, startIndex + visibleCount);
  const visibleRows = flatRows.slice(startIndex, endIndex);
  const topPad = startIndex * rowHeight;
  const bottomPad = Math.max(0, (flatRows.length - endIndex) * rowHeight);

  const selectedNodes = useMemo(() => Array.from(selectedIds).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[], [selectedIds, nodesById]);
  const selectedBytes = useMemo(() => selectedNodes.reduce((a, n) => a + BigInt(n.sizeBytes), 0n).toString(), [selectedNodes]);
  const requiresConfirmation = useMemo(() => selectedNodes.some((n) => n.risk === 'Unknown' || !n.targetId), [selectedNodes]);

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
    if (!isNodeSelectable(node)) return;
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
      setInfo('Clean completed. Review the summary and operation log below.');
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Preview Selected again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !analyzing && !previewLoading && !cleaning && (!requiresConfirmation || confirmText.trim().length > 0);
  const canClean = !!preview && !analyzing && !previewLoading && !cleaning;

  return (
    <div>
      <section className="panel" aria-labelledby="analyzer-heading">
        <h2 id="analyzer-heading">Analyzer</h2>
        <p className="muted">Analyze a folder to build an expandable tree of space usage. Select eligible nodes and route them through the standard Preview &rarr; Clean flow (no direct delete).</p>
        <div className="row">
          <label htmlFor="analyzer-root" className="muted">Root path</label>
          <input id="analyzer-root" type="text" value={rootPath} onChange={(e) => setRootPath(e.target.value)} aria-label="Analyzer root path" disabled={analyzing || cleaning} style={{ minWidth: 280 }} />
          <button className="btn primary" onClick={runAnalyzer} disabled={analyzing || cleaning || !rootPath.trim()}>Analyze</button>
        </div>
        {analyzing && (
          <div role="status" aria-live="polite" className="notice info">
            Analyzing… {progress ? `dirs ${progress.scannedDirs}, files ${progress.scannedFiles}, ${formatBytes(progress.scannedBytes)}${progress.currentPath ? ` — ${progress.currentPath}` : ''}` : 'Starting…'}
            <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
          </div>
        )}
        {sessionId && startedAt && (
          <div className="row muted" aria-label="Analyzer session status">
            <span>Session: {sessionId}</span>
            <TtlIndicator label="Analyzer session TTL (15m)" issuedAtMs={startedAt} ttlMs={SESSION_TTL_MS} />
          </div>
        )}
      </section>

      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      {result && (
        <section className="panel" aria-labelledby="analyzer-tree-heading">
          <div className="row">
            <h3 id="analyzer-tree-heading">Analyzer Tree</h3>
            <div className="spacer" />
            <span className="muted">Total: {formatBytes(result.totalBytes)} ({result.totalItems} items)</span>
            <span className="muted">Selected: {selectedIds.size} ({formatBytes(selectedBytes)})</span>
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
              const selectable = isNodeSelectable(node);
              const checked = selectedIds.has(node.id);
              const hasChildren = node.isDir && (node.children?.length ?? 0) > 0;
              const isExpanded = expanded.has(node.id);
              const reasons: string[] = [];
              if (node.risk === 'Protected') reasons.push('Protected');
              if (node.isLinkPoint) reasons.push('Link Point');
              if (node.isCloudPlaceholder) reasons.push('Cloud placeholder (Skipped)');
              if (node.displayOnly) reasons.push('Display-only');
              if (node.reason) reasons.push(node.reason);
              if (node.path.toLowerCase().endsWith('.vhdx')) reasons.push('.vhdx');
              return (
                <div
                  key={node.id}
                  role="treeitem"
                  aria-expanded={hasChildren ? isExpanded : undefined}
                  aria-selected={checked}
                  aria-level={depth + 1}
                  className="analyzer-row"
                  style={{ paddingLeft: 8 + depth * 18, height: rowHeight }}
                >
                  <button
                    type="button"
                    className="tree-toggle"
                    onClick={() => toggleExpand(node)}
                    aria-label={hasChildren ? (isExpanded ? `Collapse ${node.name}` : `Expand ${node.name}`) : `Leaf ${node.name}`}
                    disabled={!hasChildren}
                  >
                    {hasChildren ? (isExpanded ? '▾' : '▸') : '•'}
                  </button>
                  <label className="checkbox" title={selectable ? 'Select for Preview/Clean' : 'Not selectable'}>
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
                      {selectable ? 'Selectable' : `Not selectable${reasons.length ? `: ${reasons.join(', ')}` : ''}`}
                    </div>
                  </div>
                  <div className="item-size">{formatBytes(node.sizeBytes)}</div>
                  <div className="muted">{node.itemCount}</div>
                  <button className="btn" onClick={() => onExplain(node.id)}>Explain</button>
                  <span className={riskClass(node.risk)} aria-label={`Risk ${node.risk}`}>{node.risk}</span>
                  {node.tier && <span className="badge">{node.tier}</span>}
                  {node.isLinkPoint && <span className="badge">Link Point</span>}
                  {node.displayOnly && <span className="badge">Display-only</span>}
                </div>
              );
            })}
            <div style={{ height: bottomPad }} />
            {flatRows.length === 0 && <div className="muted" style={{ padding: 12 }}>No nodes.</div>}
          </div>

          <section aria-labelledby="analyzer-preview-heading" className="panel" style={{ marginTop: 12 }}>
            <h3 id="analyzer-preview-heading">Preview Selected</h3>
            <p className="muted">Selection total: <strong>{formatBytes(selectedBytes)}</strong> ({selectedIds.size} node(s)). Analyzer selections go through the standard Preview &rarr; Clean flow only.</p>
            {requiresConfirmation && (
              <div className="notice warn" role="note">
                <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>DELETE</code> to request Preview.</div>
                <div className="field">
                  <label htmlFor="analyzer-confirm-delete">Typed confirmation</label>
                  <input id="analyzer-confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder="Type DELETE" autoComplete="off" disabled={cleaning} />
                </div>
              </div>
            )}
            <div className="row"><button className="btn primary" onClick={runPreview} disabled={!canPreview}>{previewLoading ? 'Generating Preview…' : 'Preview Selected'}</button></div>
            {preview && (
              <div aria-live="polite">
                <h4 className="section-title">Preview Breakdown</h4>
                <div className="row">
                  <span>Preview token: <code>{preview.previewToken}</code></span>
                  <TtlIndicator label="Preview token TTL (10m)" issuedAtMs={preview.issuedAtMs} ttlMs={PREVIEW_TTL_MS} />
                  {startedAt && <TtlIndicator label="Analyzer session TTL (15m)" issuedAtMs={startedAt} ttlMs={SESSION_TTL_MS} />}
                </div>
                <p><strong>Total selected:</strong> {preview.totalSelectedCount} item(s), {formatBytes(preview.totalSelectedBytes)}</p>
                <table className="table" aria-label="Preview breakdown by Target">
                  <thead><tr><th>Target</th><th>Selected count</th><th>Selected bytes</th></tr></thead>
                  <tbody>
                    {preview.breakdown.map((b) => (
                      <tr key={b.targetId}><td>{b.targetName} <span className="muted">({b.targetId})</span></td><td>{b.selectedCount}</td><td>{formatBytes(b.selectedBytes)}</td></tr>
                    ))}
                  </tbody>
                </table>
                <div className="notice warn" role="alert" aria-live="polite"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div>
                <div className="row"><button className="btn danger primary" onClick={runClean} disabled={!canClean} aria-label="Clean Permanently">{cleaning ? 'Cleaning…' : 'Clean Permanently'}</button></div>
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
                {cleanSummary.logId && <div className="row" style={{ marginTop: '0.5rem' }}><button className="btn" onClick={() => onOpenLog(cleanSummary.logId!)}>View Operation Log</button><span className="muted">Log: {cleanSummary.logId}</span></div>}
              </div>
            )}
          </section>
        </section>
      )}
      <ExplainDrawer explain={explain} onClose={() => setExplain(null)} />
    </div>
  );
}

function SettingsView({ client }: { client: SweeperClient }) {
  const [settings, setSettings] = useState<Awaited<ReturnType<SweeperClient['getSettings']>> | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    client.getSettings().then(setSettings).catch((e) => setError(formatContractError(e)));
  }, [client]);
  return (
    <section className="panel" aria-labelledby="settings-heading">
      <h2 id="settings-heading">Settings</h2>
      <p className="muted">Settings are displayed for review. Editing controls remain limited in this build.</p>
      {error && <div className="notice error" role="alert">{error}</div>}
      {!settings && !error && <p>Loading settings…</p>}
      {settings && (
        <dl className="kv">
          <dt>Tier 1 enabled</dt><dd>{String(settings.enableTier1)}</dd>
          <dt>Tier 2 enabled</dt><dd>{String(settings.enableTier2)}</dd>
          <dt>Tier 3 enabled</dt><dd>{String(settings.enableTier3)}</dd>
          <dt>Minimum Age (hours)</dt><dd>{settings.minAgeHours}</dd>
          <dt>Allow deleting unknown folders</dt><dd>{String(settings.allowDeleteUnknown)}</dd>
          <dt>Custom Paths</dt><dd className="muted">Not editable in this build (read-only view; no Custom Paths configured).</dd>
        </dl>
      )}
    </section>
  );
}

function LogsView({ client, focusLogId, onFocusConsumed }: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void }) {
  const [logs, setLogs] = useState<{ id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null }[]>([]);
  const [selected, setSelected] = useState<{ id: string; content: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await client.listLogs();
      setLogs(next);
    } catch (e) {
      setError(formatContractError(e));
    }
  }, [client]);

  useEffect(() => { void refresh(); }, [refresh]);

  useEffect(() => {
    if (!focusLogId) return;
    let cancelled = false;
    (async () => {
      setError(null);
      try {
        await refresh();
        const log = await client.readLog(focusLogId);
        if (!cancelled) setSelected(log);
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) onFocusConsumed?.();
      }
    })();
    return () => { cancelled = true; };
  }, [focusLogId, client, refresh, onFocusConsumed]);

  async function openLog(id: string) {
    setError(null);
    try { setSelected(await client.readLog(id)); } catch (e) { setError(formatContractError(e)); }
  }

  return (
    <section className="panel" aria-labelledby="logs-heading">
      <h2 id="logs-heading">Logs</h2>
      <p className="muted">Operation log inspector (<code>list_logs</code> / <code>read_log</code>).</p>
      {error && <div className="notice error" role="alert">{error}</div>}
      <div className="grid-2">
        <div>
          <div className="row">
            <h3 className="section-title">Available Logs</h3>
            <div className="spacer" />
            <button className="btn" onClick={() => refresh()}>Refresh</button>
          </div>
          {logs.length === 0 ? <p className="muted">No logs found.</p> : (
            <table className="table" aria-label="Log list">
              <thead><tr><th>Name</th><th>Modified</th><th>Size</th><th></th></tr></thead>
              <tbody>
                {logs.map((l) => (
                  <tr key={l.id}>
                    <td>{l.name}</td>
                    <td>{formatUnixSeconds(l.modifiedUnixSecs)}</td>
                    <td>{l.sizeBytes ? formatBytes(l.sizeBytes) : '—'}</td>
                    <td><button className="btn" onClick={() => openLog(l.id)}>View</button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        <div>
          <h3 className="section-title">Log Content</h3>
          {selected ? (
            <div>
              <p className="muted">Log: {selected.id}</p>
              <pre className="log" aria-label="Log content">{selected.content}</pre>
            </div>
          ) : <p className="muted">Select a log to inspect its contents.</p>}
        </div>
      </div>
    </section>
  );
}

export function App() {
  const client = useMemo(() => createClient(), []);
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);

  function openLog(logId: string) {
    setFocusLogId(logId);
    setView('logs');
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">Sweeper</div>
        <nav className="nav" aria-label="Primary">
          <button aria-current={view === 'scan' ? 'page' : undefined} onClick={() => setView('scan')}>Scan &amp; Preview</button>
          <button aria-current={view === 'analyzer' ? 'page' : undefined} onClick={() => setView('analyzer')}>Analyzer</button>
          <button aria-current={view === 'settings' ? 'page' : undefined} onClick={() => setView('settings')}>Settings</button>
          <button aria-current={view === 'logs' ? 'page' : undefined} onClick={() => setView('logs')}>Logs</button>
        </nav>
      </header>
      <main className="main">
        {view === 'scan' && <ScanAndPreviewView client={client} onOpenLog={openLog} />}
        {view === 'analyzer' && <AnalyzerView client={client} onOpenLog={openLog} />}
        {view === 'settings' && <SettingsView client={client} />}
        {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} />}
      </main>
    </div>
  );
}
