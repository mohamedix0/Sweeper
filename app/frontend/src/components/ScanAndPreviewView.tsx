import { useMemo, useState } from 'react';
import {
  formatContractError,
  type CleanProgressEvent,
  type ExecuteCleanResponse,
  type ScanResults,
  type Settings,
  type SweeperClient,
} from '../api/client';
import type { ContractError, TargetScanResultDto } from '../contract/bindings';
import { formatBytes, sumDecimalStrings } from '../utils/format';
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

export function ScanAndPreviewView({ client, settings, onOpenLog }: { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void }) {
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
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

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

  const requiresConfirmation = useMemo(() => scanSelectionNeedsConfirmation(targets, selectedIds), [targets, selectedIds]);

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

  function toggleItem(itemId: string, target: TargetScanResultDto, ageEligible: boolean) {
    if (!isSelectableScanItem({ ageEligible }, target, allowDeleteUnknown)) return;
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
      setConfirmText('');
      setInfo('Clean completed. Review the summary and operation log below.');
      if (sessionId) { try { setResults(await client.getScanResults(sessionId)); } catch { /* ignore */ } }
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetForInvalidSession('ScanSession expired or was invalidated. Start a new Scan.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Preview again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading && !cleaning && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
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
                  const selectable = isSelectableScanItem(item, t, allowDeleteUnknown);
                  const checked = selectedIds.has(item.id);
                  const reason = t.risk === 'Protected'
                    ? 'Protected — never selectable'
                    : (t.risk === 'Unknown' && !allowDeleteUnknown)
                      ? 'Unknown — gated by Settings (Allow deleting unknown folders)'
                      : (!item.ageEligible ? 'Not eligible (Minimum Age / recent or ineligible)' : 'Eligible');
                  return (
                    <div key={item.id} className="item-row">
                      <label className="checkbox">
                        <input type="checkbox" aria-label={`Select ${item.path}`} checked={checked} disabled={!selectable || cleaning} onChange={() => toggleItem(item.id, t, item.ageEligible)} />
                      </label>
                      <div>
                        <div className="item-path">{item.path}</div>
                        <div className="item-status">{!selectable ? `Skipped/Not selectable: ${reason}` : (t.risk === 'Unknown' || isCustomPathTargetId(t.targetId) ? 'Eligible (typed confirmation required at Preview)' : 'Eligible for Preview selection')}</div>
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
                <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path items). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
                <div className="field">
                  <label htmlFor="confirm-delete">Typed confirmation</label>
                  <input id="confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={cleaning} />
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
