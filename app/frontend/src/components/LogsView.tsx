import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatContractError, type SweeperClient } from '../api/client';
import { formatBytes, formatUnixSeconds, pluralize } from '../utils/format';
import { CopyIcon, ExportIcon, RefreshIcon } from './Icons';

type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
type AuditRow = { time: string; category: string; location: string; size: string; outcome: string; outcomeTone: 'eligible' | 'cleaned' | 'skipped' | 'neutral' };
type LogFilter = 'all' | 'scan' | 'clean';

function formatLogWhen(meta: LogMeta): string {
  if (!meta.modifiedUnixSecs) return '—';
  try {
    const n = Number(meta.modifiedUnixSecs);
    if (!Number.isFinite(n)) return formatUnixSeconds(meta.modifiedUnixSecs);
    const d = new Date(n * (n < 1e12 ? 1000 : 1));
    if (Number.isNaN(d.getTime())) return formatUnixSeconds(meta.modifiedUnixSecs);
    return d.toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' });
  } catch {
    return formatUnixSeconds(meta.modifiedUnixSecs);
  }
}

function formatTime(raw: unknown, fallbackMeta: LogMeta | null): string {
  if (typeof raw === 'string') {
    const n = Number(raw);
    if (Number.isFinite(n) && n > 1e9) {
      const d = new Date(n * (n < 1e12 ? 1000 : 1));
      if (!Number.isNaN(d.getTime())) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
    if (/^\d{1,2}:\d{2}/.test(raw.trim())) return raw.trim();
  }
  if (typeof raw === 'number' && Number.isFinite(raw) && raw > 1e9) {
    const d = new Date(raw * (raw < 1e12 ? 1000 : 1));
    if (!Number.isNaN(d.getTime())) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  }
  if (fallbackMeta?.modifiedUnixSecs) {
    const n = Number(fallbackMeta.modifiedUnixSecs);
    if (Number.isFinite(n)) {
      const d = new Date(n * (n < 1e12 ? 1000 : 1));
      if (!Number.isNaN(d.getTime())) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
  }
  return '—';
}

function normalizeOutcome(raw: unknown, fallback: 'eligible' | 'cleaned' | 'skipped' | 'neutral'): { label: string; tone: AuditRow['outcomeTone'] } {
  const s = String(raw ?? '').toLowerCase();
  if (s.includes('clean')) return { label: 'Cleaned', tone: 'cleaned' };
  if (s.includes('eligible')) return { label: 'Eligible', tone: 'eligible' };
  if (s.includes('skip')) {
    if (s.includes('< 24') || s.includes('24h')) return { label: 'Skipped · < 24h', tone: 'skipped' };
    return { label: 'Skipped', tone: 'skipped' };
  }
  if (fallback === 'cleaned') return { label: 'Cleaned', tone: 'cleaned' };
  if (fallback === 'eligible') return { label: 'Eligible', tone: 'eligible' };
  if (fallback === 'skipped') return { label: 'Skipped', tone: 'skipped' };
  return { label: String(raw || 'Recorded'), tone: 'neutral' };
}

function parseAuditRows(content: string, meta: LogMeta | null): AuditRow[] {
  const lines = content.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  const objs: Record<string, unknown>[] = [];
  for (const line of lines) {
    try { objs.push(JSON.parse(line) as Record<string, unknown>); } catch { /* ignore raw noise in summary */ }
  }
  if (objs.length === 0) return [];

  const outcome = [...objs].reverse().find((o) => o.record === 'outcome') ?? null;
  const intent = objs.find((o) => o.record === 'intent') ?? null;
  const isClean = !!(outcome?.event === 'clean' || outcome?.action === 'clean' || intent?.action === 'clean' || outcome?.deletedCount != null || outcome?.bytesFreed != null);
  const isScan = !isClean && !!(outcome?.event === 'scan' || outcome?.action === 'scan' || intent?.action === 'scan' || outcome?.totalBytes != null || outcome?.categoriesCount != null);

  const itemRows = objs.filter((o) => {
    const rec = String(o.record ?? '');
    const ev = String(o.event ?? o.action ?? '').toLowerCase();
    if (rec === 'outcome' || rec === 'intent') return false;
    if (ev === 'scan' && (o.categoriesCount != null || o.totalBytes != null)) return false;
    if (o.category || o.path || o.target || o.size || o.bytes || o.status) return true;
    return false;
  });

  if (itemRows.length > 0) {
    return itemRows.map((o) => {
      const time = formatTime(o.timestamp ?? o.time ?? o.ts, meta);
      const category = String(o.category ?? o.target ?? o.action ?? (isScan ? 'Scan' : 'Cleanup'));
      const location = String(o.path ?? o.details ?? o.location ?? '—');
      const rawSize = o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes;
      const size = rawSize != null && /^\d+$/.test(String(rawSize)) ? formatBytes(String(rawSize)) : (rawSize ? String(rawSize) : '—');
      const outcomeNorm = normalizeOutcome(o.status ?? (isClean ? 'Cleaned' : isScan ? 'Eligible' : 'Recorded'), isClean ? 'cleaned' : isScan ? 'eligible' : 'neutral');
      return { time, category, location, size, outcome: outcomeNorm.label, outcomeTone: outcomeNorm.tone };
    });
  }

  // Fallback: synthesize clean category-level/outcome rows from outcome/intent
  const time = formatTime(outcome?.timestamp ?? intent?.timestamp, meta);
  if (isClean) {
    const category = String(outcome?.category ?? intent?.category ?? 'Cleanup');
    const deleted = outcome?.deletedCount != null ? Number(outcome.deletedCount) : null;
    const freed = outcome?.bytesFreed != null ? formatBytes(String(outcome.bytesFreed)) : '—';
    return [{
      time,
      category,
      location: deleted != null ? `${deleted} ${deleted === 1 ? 'item' : 'items'} removed` : 'Completed cleanup',
      size: freed,
      outcome: 'Cleaned',
      outcomeTone: 'cleaned',
    }];
  }
  if (isScan) {
    const cats = outcome?.categoriesCount ?? intent?.categoriesCount;
    const total = outcome?.totalBytes ?? intent?.totalBytes;
    const items = outcome?.items ?? intent?.items;
    return [{
      time,
      category: 'System Scan',
      location: `${cats ?? '—'} ${Number(cats) === 1 ? 'category' : 'categories'}${items != null ? ` · ${items} items` : ''}`,
      size: total != null ? formatBytes(String(total)) : '—',
      outcome: 'Eligible',
      outcomeTone: 'eligible',
    }];
  }
  return [{ time, category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral' }];
}

function summarizeLog(meta: LogMeta, content: string | null): { title: string; subtitle: string; when: string; kind: LogFilter } {
  const when = formatLogWhen(meta);
  if (content) {
    try {
      const lines = content.split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l) as Record<string, unknown>);
      const outcome = [...lines].reverse().find((o) => o.record === 'outcome' || o.event === 'clean' || o.event === 'scan' || o.action === 'scan' || o.action === 'clean');
      if (outcome) {
        const isClean = String(outcome.record) === 'outcome' && (outcome.event === 'clean' || outcome.action === 'clean' || outcome.deletedCount != null || outcome.bytesFreed != null) && !(String(outcome.event) === 'scan' || outcome.action === 'scan');
        const isScan = String(outcome.event) === 'scan' || outcome.action === 'scan' || (String(outcome.record) === 'outcome' && (outcome.categoriesCount != null || outcome.totalBytes != null) && outcome.deletedCount == null);
        if (isClean) {
          const deleted = outcome.deletedCount ?? outcome.deleted;
          const deletedStr = deleted !== undefined && deleted !== null ? `${String(deleted)} item${String(deleted) === '1' ? '' : 's'}` : null;
          const hasBytes = typeof outcome.bytesFreed === 'string' || typeof outcome.bytesFreed === 'number';
          const freed = hasBytes ? formatBytes(String(outcome.bytesFreed)) : null;
          if (freed && deletedStr) return { title: `Cleaned ${freed} · ${deletedStr}`, subtitle: 'Completed', when, kind: 'clean' };
          if (deletedStr) return { title: `Cleaned ${deletedStr}`, subtitle: 'Completed', when, kind: 'clean' };
          if (freed) return { title: `Cleaned ${freed}`, subtitle: 'Completed', when, kind: 'clean' };
          return { title: 'Clean Operation', subtitle: 'Completed', when, kind: 'clean' };
        }
        if (isScan) {
          const rawCategories = outcome.categoriesCount;
          const categoryCount = rawCategories != null ? Number(rawCategories) : null;
          const detail = categoryCount && categoryCount > 0 ? ` · ${categoryCount} ${categoryCount === 1 ? 'category' : 'categories'}` : '';
          return { title: `System Scan${detail}`, subtitle: 'Completed', when, kind: 'scan' };
        }
      }
    } catch { /* fall through */ }
  }
  const lower = meta.name.toLowerCase();
  if (lower.includes('clean')) return { title: 'Clean Operation', subtitle: 'Completed', when, kind: 'clean' };
  if (lower.includes('scan')) return { title: 'System Scan', subtitle: 'Completed', when, kind: 'scan' };
  return { title: meta.name.replace(/\.jsonl$/i, ''), subtitle: 'Completed', when, kind: 'all' };
}

function deriveActivitySummary(content: string | null): { primaryLabel: string; primaryValue: string; categoriesItems: string; outcome: string } {
  if (!content) return { primaryLabel: 'Space Reclaimed', primaryValue: '—', categoriesItems: '—', outcome: '—' };
  try {
    const lines = content.split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l) as Record<string, unknown>);
    const outcome = [...lines].reverse().find((o) => o.record === 'outcome') ?? [...lines].reverse()[0];
    const intent = lines.find((o) => o.record === 'intent') ?? lines[0];
    const isClean = !!(outcome?.deletedCount != null || outcome?.bytesFreed != null || outcome?.event === 'clean' || intent?.action === 'clean');
    const isScan = !isClean && (outcome?.event === 'scan' || intent?.action === 'scan' || outcome?.categoriesCount != null || (outcome?.totalBytes != null && outcome?.deletedCount == null));
    let primaryLabel = 'Space Reclaimed';
    let primaryValue = '0 B';
    if (isClean) {
      primaryLabel = 'Space Reclaimed';
      if (typeof outcome?.bytesFreed === 'string' || typeof outcome?.bytesFreed === 'number') primaryValue = formatBytes(String(outcome.bytesFreed));
    } else if (isScan) {
      primaryLabel = 'Space Found';
      if (typeof outcome?.totalBytes === 'string' || typeof outcome?.totalBytes === 'number') primaryValue = formatBytes(String(outcome.totalBytes));
    }
    const categoriesCount = outcome?.categoriesCount != null ? Number(outcome.categoriesCount) : (intent?.categoriesCount != null ? Number(intent.categoriesCount) : null);
    const itemsCount = outcome?.items != null ? Number(outcome.items) : (isClean && outcome?.deletedCount != null ? Number(outcome.deletedCount) : null);
    let categoriesItems = '—';
    if (categoriesCount != null && itemsCount != null) categoriesItems = `${categoriesCount} ${categoriesCount === 1 ? 'category' : 'categories'} · ${itemsCount} items`;
    else if (categoriesCount != null) categoriesItems = `${categoriesCount} ${categoriesCount === 1 ? 'category' : 'categories'}`;
    else if (itemsCount != null) categoriesItems = `${itemsCount} items`;
    const status = String(outcome?.status ?? 'completed');
    return { primaryLabel, primaryValue, categoriesItems, outcome: status === 'completed' ? 'Completed' : status };
  } catch {
    return { primaryLabel: 'Space Reclaimed', primaryValue: '—', categoriesItems: '—', outcome: 'Recorded' };
  }
}

export function LogsView({ client, focusLogId, onFocusConsumed }: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void }) {
  const [logs, setLogs] = useState<LogMeta[]>([]);
  const [contentById, setContentById] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<{ id: string; content: string } | null>(null);
  const [mode, setMode] = useState<'structured' | 'raw'>('structured');
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [filter, setFilter] = useState<LogFilter>('all');
  const [query, setQuery] = useState('');

  const refresh = useCallback(async (): Promise<LogMeta[]> => {
    try {
      const next = await client.listLogs();
      setLogs(next);
      void Promise.all(
        next.slice(0, 15).map(async (item) => {
          try { const loaded = await client.readLog(item.id); return [item.id, loaded.content] as const; } catch { return null; }
        }),
      ).then((entries) => {
        const map: Record<string, string> = {};
        for (const entry of entries) if (entry) map[entry[0]] = entry[1];
        if (Object.keys(map).length > 0) setContentById((prev) => ({ ...prev, ...map }));
      });
      return next;
    } catch (e) {
      setError(formatContractError(e));
      return [];
    }
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next = await refresh();
      if (cancelled) return;
      if (!focusLogId && !selected && next.length > 0) {
        try {
          const log = await client.readLog(next[0].id);
          if (!cancelled) { setSelected(log); setContentById((prev) => ({ ...prev, [log.id]: log.content })); }
        } catch (e) { if (!cancelled) setError(formatContractError(e)); }
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);

  useEffect(() => {
    if (!focusLogId) return;
    let cancelled = false;
    (async () => {
      setError(null);
      try {
        await refresh();
        const log = await client.readLog(focusLogId);
        if (!cancelled) { setSelected(log); setContentById((prev) => ({ ...prev, [log.id]: log.content })); }
      } catch (e) { if (!cancelled) setError(formatContractError(e)); }
      finally { if (!cancelled) onFocusConsumed?.(); }
    })();
    return () => { cancelled = true; };
  }, [focusLogId, client, refresh, onFocusConsumed]);

  async function openLog(id: string) {
    setError(null);
    try {
      const log = await client.readLog(id);
      setSelected(log);
      setContentById((prev) => ({ ...prev, [log.id]: log.content }));
    } catch (e) { setError(formatContractError(e)); }
  }

  const selectedMeta = useMemo(() => selected ? logs.find((l) => l.id === selected.id) ?? null : null, [logs, selected]);
  const auditRows = useMemo(() => (selected ? parseAuditRows(selected.content, selectedMeta) : []), [selected, selectedMeta]);
  const selectedSummary = useMemo(() => selectedMeta ? summarizeLog(selectedMeta, selected?.content ?? null) : null, [selectedMeta, selected]);
  const activitySummary = useMemo(() => deriveActivitySummary(selected?.content ?? null), [selected]);

  const q = query.trim().toLowerCase();
  const filteredLogs = useMemo(() => {
    return logs.filter((l) => {
      const sum = summarizeLog(l, (selected?.id === l.id ? selected.content : contentById[l.id]) ?? null);
      if (filter !== 'all' && sum.kind !== filter) return false;
      if (!q) return true;
      const content = (selected?.id === l.id ? selected.content : contentById[l.id]) ?? '';
      return sum.title.toLowerCase().includes(q) || sum.when.toLowerCase().includes(q) || content.toLowerCase().includes(q) || l.name.toLowerCase().includes(q);
    });
  }, [logs, contentById, selected, filter, q]);

  async function copyLog() {
    if (!selected) return;
    try { await navigator.clipboard?.writeText(selected.content); setCopied(true); setTimeout(() => setCopied(false), 1500); } catch { /* ignore */ }
  }

  function exportLog() {
    if (!selected) return;
    const blob = new Blob([selected.content], { type: 'application/x-ndjson' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = selectedMeta?.name || `${selected.id}.jsonl`;
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header" aria-label="Activity History header">
        <div className="page-header-left">
          <h1 className="page-title" id="logs-heading">Activity History</h1>
          <span className="hero-metric-badge num">{pluralize(logs.length, 'activity', 'activities')}</span>
        </div>
        <div className="page-header-right">
          <button className="btn secondary-action" onClick={() => { void refresh(); }} aria-label="Refresh"><RefreshIcon /><span>Refresh</span></button>
        </div>
      </header>

      {error && <div className="notice error" role="alert">{error}</div>}

      <section className="logs-layout" aria-label="Activity history">
        <div className="logs-pane logs-pane-left">
          <div className="logs-pane-header logs-pane-header-stack">
            <h2 className="pane-section-title">Recent Activity</h2>
            <div className="logs-filter-bar">
              <div className="toolbar-chip-group" role="group" aria-label="Activity filters">
                <button type="button" className={`chip-btn ${filter === 'all' ? 'active' : ''}`} onClick={() => setFilter('all')}>All</button>
                <button type="button" className={`chip-btn ${filter === 'scan' ? 'active' : ''}`} onClick={() => setFilter('scan')}>Scans</button>
                <button type="button" className={`chip-btn ${filter === 'clean' ? 'active' : ''}`} onClick={() => setFilter('clean')}>Cleanups</button>
              </div>
              <input type="search" className="logs-search" placeholder="Search activity..." value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search activity history" />
            </div>
          </div>
          <div className="logs-pane-body">
            {logs.length === 0 ? <p className="muted pad-md">No activity yet.</p> : filteredLogs.length === 0 ? <p className="muted pad-md">No matching activity.</p> : (
              <ul className="log-list" aria-label="Log list">
                {filteredLogs.map((l) => {
                  const sum = summarizeLog(l, (selected?.id === l.id ? selected.content : contentById[l.id]) ?? null);
                  const active = selected?.id === l.id;
                  return (
                    <li key={l.id} className="log-list-item">
                      <button type="button" className={`log-select-btn ${active ? 'active' : ''}`} onClick={() => openLog(l.id)}>
                        <span className="log-primary">{sum.title}</span>
                        <span className="log-secondary"><span>{sum.when}</span><span className="log-status-badge status-completed">{sum.subtitle}</span></span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </div>

        <div className="logs-pane logs-pane-right">
          <div className="logs-pane-header log-content-header">
            <h2 className="pane-section-title">Activity Details</h2>
            <div className="spacer" />
            {selected && (
              <div className="log-header-actions">
                <div className="segmented-toggle" role="group" aria-label="Log view mode">
                  <button type="button" className={`seg-btn ${mode === 'structured' ? 'active' : ''}`} aria-pressed={mode === 'structured'} onClick={() => setMode('structured')}>Summary</button>
                  <button type="button" className={`seg-btn ${mode === 'raw' ? 'active' : ''}`} aria-pressed={mode === 'raw'} aria-label="Raw JSONL" onClick={() => setMode('raw')}>Raw Log</button>
                </div>
                <button type="button" className="btn small subtle-btn" onClick={() => void copyLog()} aria-label={copied ? 'Copied' : 'Copy log'} title="Copy"><CopyIcon /><span>{copied ? 'Copied' : 'Copy'}</span></button>
                <button type="button" className="btn small subtle-btn" onClick={exportLog} aria-label="Export log" title="Export"><ExportIcon /><span>Export</span></button>
              </div>
            )}
          </div>
          <div className="logs-pane-body log-details">
            {selected ? (
              <>
                <div aria-label="Structured log summary" className="log-summary-head">
                  {selectedSummary && <div className="log-summary-title">{selectedSummary.title}</div>}
                  {selectedSummary && <div className="muted log-summary-meta"><span>{selectedSummary.when}</span><span className="log-status-badge status-completed">{selectedSummary.subtitle}</span></div>}
                </div>
                <div className="activity-summary-strip" aria-label="Activity summary">
                  <div className="activity-summary-cell"><span className="activity-summary-label">{activitySummary.primaryLabel}</span><span className="activity-summary-value num">{activitySummary.primaryValue}</span></div>
                  <div className="activity-summary-cell"><span className="activity-summary-label">Categories</span><span className="activity-summary-value">{activitySummary.categoriesItems}</span></div>
                  <div className="activity-summary-cell"><span className="activity-summary-label">Outcome</span><span className="activity-summary-value">{activitySummary.outcome}</span></div>
                </div>
                {mode === 'structured' ? (
                  <div className="log-structured audit-table-wrap">
                    {auditRows.length === 0 ? <p className="muted">No entries.</p> : (
                      <table className="table audit-log-table">
                        <thead><tr><th>Time</th><th>Category / Target</th><th>Location</th><th className="num">Size</th><th>Outcome</th></tr></thead>
                        <tbody>
                          {auditRows.map((r, idx) => (
                            <tr key={idx}>
                              <td className="audit-time num">{r.time}</td>
                              <td className="audit-category">{r.category}</td>
                              <td className="audit-location mono" title={r.location}>{r.location}</td>
                              <td className="audit-size num">{r.size}</td>
                              <td className="audit-outcome"><span className={`badge outcome-pill outcome-${r.outcomeTone}`}>{r.outcome}</span></td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </div>
                ) : (
                  <pre className="log" aria-label="Log content">{selected.content}</pre>
                )}
              </>
            ) : <p className="muted pad-md">Select an activity entry to inspect details.</p>}
          </div>
        </div>
      </section>
    </div>
  );
}
