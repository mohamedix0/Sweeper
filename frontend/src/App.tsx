import { useEffect, useMemo, useState } from 'react';
import './App.css';
import { createClient, formatContractError, type ScanResults, type SweeperClient } from './api/client';
import type { ContractError, RiskDto, TargetScanResultDto } from './contract/bindings';
import { formatBytes, formatUnixSeconds, sumDecimalStrings } from './utils/format';

type View = 'scan' | 'settings' | 'logs';

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
  // Unknown items can be displayed; selection is allowed in UI flow but requires typed confirmation at preview
  return true;
}

function needsTypedConfirmation(targets: TargetScanResultDto[], selectedIds: Set<string>): boolean {
  for (const t of targets) {
    if (t.risk === 'Unknown') {
      if (t.items.some((i) => selectedIds.has(i.id))) return true;
    }
    for (const item of t.items) {
      if (!selectedIds.has(item.id)) continue;
      // Custom Path targets would be indicated by targetId prefix in real data; mock uses custom-
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

function ScanAndPreviewView({ client }: { client: SweeperClient }) {
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
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const targets = useMemo(() => results?.targets ?? [], [results]);

  const selectedBytes = useMemo(() => {
    const sizes: string[] = [];
    for (const t of targets) {
      for (const item of t.items) {
        if (selectedIds.has(item.id)) sizes.push(item.size);
      }
    }
    return sumDecimalStrings(sizes);
  }, [targets, selectedIds]);

  const selectableCount = useMemo(() => {
    let n = 0;
    for (const t of targets) for (const i of t.items) if (isSelectableItem(i, t.risk)) n++;
    return n;
  }, [targets]);

  const requiresConfirmation = useMemo(() => needsTypedConfirmation(targets, selectedIds), [targets, selectedIds]);

  async function runScan() {
    setError(null);
    setInfo(null);
    setPreview(null);
    setExplain(null);
    setSelectedIds(new Set());
    setResults(null);
    setScanning(true);
    setScanStatus('Starting Scan…');
    const tiers = [tier1 ? 'Tier1' : null, tier2 ? 'Tier2' : null].filter(Boolean) as ('Tier1' | 'Tier2')[];
    try {
      const { sessionId: sid } = await client.startScan(tiers.length ? tiers : null, (ev) => {
        if (ev.kind === 'started') setScanStatus('Scan in progress…');
        if (ev.kind === 'completed') setScanStatus('Scan completed.');
        if (ev.kind === 'failed') setScanStatus('Scan failed.');
      });
      setSessionId(sid);
      const started = Date.now();
      setScanStartedAt(started);
      const data = await client.getScanResults(sid);
      setResults(data);
      setInfo('Scan complete. Select eligible items and run Preview. No files are removed in Phase 2.');
    } catch (e) {
      setError(formatContractError(e));
      if ((e as ContractError | undefined)?.code === 'sessionInvalid') {
        setSessionId(null);
        setResults(null);
      }
    } finally {
      setScanning(false);
    }
  }

  function toggleItem(itemId: string, targetRisk: RiskDto, ageEligible: boolean) {
    if (!isSelectableItem({ ageEligible }, targetRisk)) return;
    setPreview(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(itemId)) next.delete(itemId);
      else next.add(itemId);
      return next;
    });
  }

  async function onExplain(itemId: string) {
    setError(null);
    try {
      const res = await client.explainItem(itemId);
      setExplain(res);
    } catch (e) {
      setError(formatContractError(e));
    }
  }

  async function runPreview() {
    if (!sessionId) return;
    setError(null);
    setInfo(null);
    setPreviewLoading(true);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(sessionId, Array.from(selectedIds), confirmations);
      setPreview({ ...res, issuedAtMs: Date.now() });
      setInfo('Preview ready. Review the breakdown below. Clean is locked in Phase 2 (no permanent deletion).');
    } catch (e) {
      setError(formatContractError(e));
      const ce = e as ContractError | undefined;
      if (ce?.code === 'sessionInvalid' || ce?.code === 'previewTokenInvalid') {
        setPreview(null);
        if (ce.code === 'sessionInvalid') {
          setSessionId(null);
          setResults(null);
          setSelectedIds(new Set());
        }
      }
    } finally {
      setPreviewLoading(false);
    }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !scanning && !previewLoading &&
    (!requiresConfirmation || confirmText.trim().length > 0);

  return (
    <div>
      <section className="panel" aria-labelledby="scan-heading">
        <h2 id="scan-heading">Scan &amp; Preview</h2>
        <p className="muted">Read-only Phase 2: discover reclaimable Targets, explain items, and generate a Preview. No files are removed.</p>
        <div className="row">
          <label className="checkbox"><input type="checkbox" checked={tier1} onChange={(e) => setTier1(e.target.checked)} /> Tier 1 (system temp and caches)</label>
          <label className="checkbox"><input type="checkbox" checked={tier2} onChange={(e) => setTier2(e.target.checked)} /> Tier 2 (developer caches)</label>
          <div className="spacer" />
          <button className="btn primary" onClick={runScan} disabled={scanning}>Start Scan</button>
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
                        <input
                          type="checkbox"
                          aria-label={`Select ${item.path}`}
                          checked={checked}
                          disabled={!selectable}
                          onChange={() => toggleItem(item.id, t.risk, item.ageEligible)}
                        />
                      </label>
                      <div>
                        <div className="item-path">{item.path}</div>
                        <div className="item-status">
                          {!selectable ? `Skipped/Not selectable: ${reason}` : 'Eligible for Preview selection'}
                        </div>
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
                  <input id="confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder="Type DELETE" autoComplete="off" />
                </div>
              </div>
            )}
            <div className="row">
              <button className="btn primary" onClick={runPreview} disabled={!canPreview}>
                {previewLoading ? 'Generating Preview…' : 'Preview Clean (read-only)'}
              </button>
              <button className="btn" disabled title="Clean is available in Phase 3 only">Clean (Phase 3 — locked)</button>
            </div>
            <div className="notice info" role="status">
              <strong>Phase 2 notice:</strong> Permanent Clean is not available yet. This build never calls <code>execute_clean</code> and performs no deletions. Deletion arrives in Phase 3 after Preview + confirmation.
            </div>

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
                  <thead>
                    <tr><th>Target</th><th>Selected count</th><th>Selected bytes</th></tr>
                  </thead>
                  <tbody>
                    {preview.breakdown.map((b) => (
                      <tr key={b.targetId}>
                        <td>{b.targetName} <span className="muted">({b.targetId})</span></td>
                        <td>{b.selectedCount}</td>
                        <td>{formatBytes(b.selectedBytes)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </section>
      )}

      {explain && (
        <div className="drawer-backdrop" role="presentation" onClick={() => setExplain(null)}>
          <aside
            className="drawer"
            role="dialog"
            aria-modal="true"
            aria-labelledby="explain-title"
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setExplain(null);
            }}
          >
            <div className="row">
              <h3 id="explain-title">Explain: {explain.title}</h3>
              <div className="spacer" />
              <button className="btn" onClick={() => setExplain(null)} aria-label="Close explanation">Close</button>
            </div>
            <p><span className={riskClass(explain.risk)}>Risk: {explain.risk}</span></p>
            <p>{explain.description}</p>
            <dl className="kv">
              <dt>Item ID</dt><dd>{explain.itemId}</dd>
            </dl>
          </aside>
        </div>
      )}
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
      <h2 id="settings-heading">Settings (Read-Only in Phase 2)</h2>
      <p className="muted">Settings are displayed for review. Editing and <code>update_settings</code> are not enabled in this phase.</p>
      {error && <div className="notice error" role="alert">{error}</div>}
      {!settings && !error && <p>Loading settings…</p>}
      {settings && (
        <dl className="kv">
          <dt>Tier 1 enabled</dt><dd>{String(settings.enableTier1)}</dd>
          <dt>Tier 2 enabled</dt><dd>{String(settings.enableTier2)}</dd>
          <dt>Tier 3 enabled</dt><dd>{String(settings.enableTier3)}</dd>
          <dt>Minimum Age (hours)</dt><dd>{settings.minAgeHours}</dd>
          <dt>Allow deleting unknown folders</dt><dd>{String(settings.allowDeleteUnknown)}</dd>
          <dt>Custom Paths</dt><dd className="muted">Not editable in Phase 2 (read-only view; no Custom Paths configured).</dd>
        </dl>
      )}
    </section>
  );
}

function LogsView({ client }: { client: SweeperClient }) {
  const [logs, setLogs] = useState<{ id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null }[]>([]);
  const [selected, setSelected] = useState<{ id: string; content: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    client.listLogs().then(setLogs).catch((e) => setError(formatContractError(e)));
  }, [client]);
  async function openLog(id: string) {
    setError(null);
    try {
      setSelected(await client.readLog(id));
    } catch (e) {
      setError(formatContractError(e));
    }
  }
  return (
    <section className="panel" aria-labelledby="logs-heading">
      <h2 id="logs-heading">Logs</h2>
      <p className="muted">Read-only log inspector (<code>list_logs</code> / <code>read_log</code>).</p>
      {error && <div className="notice error" role="alert">{error}</div>}
      <div className="grid-2">
        <div>
          <h3 className="section-title">Available Logs</h3>
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

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">Sweeper</div>
        <nav className="nav" aria-label="Primary">
          <button aria-current={view === 'scan' ? 'page' : undefined} onClick={() => setView('scan')}>Scan &amp; Preview</button>
          <button aria-current={view === 'settings' ? 'page' : undefined} onClick={() => setView('settings')}>Settings (Read-Only)</button>
          <button aria-current={view === 'logs' ? 'page' : undefined} onClick={() => setView('logs')}>Logs</button>
        </nav>
      </header>
      <main className="main">
        {view === 'scan' && <ScanAndPreviewView client={client} />}
        {view === 'settings' && <SettingsView client={client} />}
        {view === 'logs' && <LogsView client={client} />}
      </main>
    </div>
  );
}
