import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { formatContractError, type SweeperClient } from '../api/client';
import { formatBytes, formatUnixSeconds, pluralize, relativeAge } from '../utils/format';
import { CheckIcon, CopyIcon, ExportIcon, RefreshIcon } from './Icons';

type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
type AuditRow = { type: string; category: string; location: string; size: string; outcome: string; outcomeTone: 'eligible' | 'cleaned' | 'skipped' | 'neutral'; kind: 'Category' | 'File'; sublabel?: string };
type LogFilter = 'all' | 'scan' | 'clean';
type OutcomeFilter = 'all' | 'eligible' | 'skipped';


function formatClockTime(meta: LogMeta | null, raw?: unknown): string {
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
  if (meta?.modifiedUnixSecs) {
    const n = Number(meta.modifiedUnixSecs);
    if (Number.isFinite(n)) {
      const d = new Date(n * (n < 1e12 ? 1000 : 1));
      if (!Number.isNaN(d.getTime())) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
  }
  return '';
}



function formatLogWhen(meta: LogMeta): string {
  if (!meta.modifiedUnixSecs) return '—';
  const rel = relativeAge(meta.modifiedUnixSecs);
  if (rel !== '—') return rel;
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

function normalizeOutcome(raw: unknown, fallback: 'eligible' | 'cleaned' | 'skipped' | 'neutral'): { label: string; tone: AuditRow['outcomeTone'] } {
  const s = String(raw ?? '').toLowerCase();
  if (s.includes('clean')) return { label: 'Cleaned', tone: 'cleaned' };
  if (s.includes('eligible') || s.includes('safe')) return { label: 'Safe', tone: 'eligible' };
  if (s.includes('skip')) {
    if (s.includes('unknown') || s.includes('unverified')) return { label: 'Skipped · Unverified', tone: 'skipped' };
    if (s.includes('< 24') || s.includes('24h')) return { label: 'Skipped · < 24h', tone: 'skipped' };
    return { label: 'Skipped', tone: 'skipped' };
  }
  if (fallback === 'cleaned') return { label: 'Cleaned', tone: 'cleaned' };
  if (fallback === 'eligible') return { label: 'Safe', tone: 'eligible' };
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
      const rawCategory = String(o.category ?? o.target ?? o.action ?? (isScan ? 'Scan' : 'Cleanup'));
      const location = String(o.path ?? o.details ?? o.location ?? '—');
      const rawSize = o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes;
      const size = rawSize != null && /^\d+$/.test(String(rawSize)) ? formatBytes(String(rawSize)) : (rawSize ? String(rawSize) : '—');
      const outcomeNorm = normalizeOutcome(o.status ?? (isClean ? 'Cleaned' : isScan ? 'Eligible' : 'Recorded'), isClean ? 'cleaned' : isScan ? 'eligible' : 'neutral');
      const explicitKind = o.kind === 'Category' || o.kind === 'File' ? (o.kind as 'Category' | 'File') : null;
      const kind = explicitKind ?? 'File';
      let category = rawCategory;
      let sublabel: string | undefined = kind === 'Category' ? undefined : (outcomeNorm.tone === 'skipped' ? 'Skipped file' : 'File');
      if (kind === 'File' && outcomeNorm.tone === 'skipped' && location && location !== '—') {
        const parts = location.split(/[\\/]/).filter(Boolean);
        const fileName = parts[parts.length - 1] || rawCategory;
        category = fileName;
        const parentLabel = rawCategory === 'Custom Path (Unknown)' ? `Custom Path · ${(parts[parts.length - 2] || 'CustomCleanup')}` : rawCategory;
        sublabel = parentLabel;
      } else if (kind === 'File' && outcomeNorm.tone !== 'skipped') {
        sublabel = undefined;
      }
      const type = outcomeNorm.tone === 'skipped' ? 'Skipped file' : 'Category';
      return { type, category, location, size, outcome: outcomeNorm.label, outcomeTone: outcomeNorm.tone, kind, sublabel };
    });
  }

  // Fallback: synthesize clean category-level/outcome rows from outcome/intent
  if (isClean) {
    const category = String(outcome?.category ?? intent?.category ?? 'Cleanup');
    const deleted = outcome?.deletedCount != null ? Number(outcome.deletedCount) : null;
    const freed = outcome?.bytesFreed != null ? formatBytes(String(outcome.bytesFreed)) : '—';
    return [{
      type: 'Category',
      category,
      location: deleted != null ? `${deleted} ${deleted === 1 ? 'item' : 'items'} removed` : 'Completed cleanup',
      size: freed,
      outcome: 'Cleaned',
      outcomeTone: 'cleaned',
      kind: 'Category',
      sublabel: 'Category',
    }];
  }
  if (isScan) {
    const cats = outcome?.categoriesCount ?? intent?.categoriesCount;
    const total = outcome?.totalBytes ?? intent?.totalBytes;
    const items = outcome?.items ?? intent?.items;
    return [{
      type: 'Category',
      category: 'System Scan',
      location: `${cats ?? '—'} ${Number(cats) === 1 ? 'category' : 'categories'}${items != null ? ` · ${items} items` : ''}`,
      size: total != null ? formatBytes(String(total)) : '—',
      outcome: 'Safe',
      outcomeTone: 'eligible',
      kind: 'Category',
      sublabel: 'Category',
    }];
  }
  return [{ type: 'Category', category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral', kind: 'Category', sublabel: 'Category' }];
}

function summarizeLog(meta: LogMeta, content: string | null): { title: string; subtitle: string; when: string; whenAt: string; kind: LogFilter } {
  const when = formatLogWhen(meta);
  const clock = formatClockTime(meta);
  const whenAt = clock ? `${when} at ${clock}` : when;
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
          if (freed && deletedStr) return { title: `Cleaned ${freed} · ${deletedStr}`, subtitle: 'Completed', when, whenAt, kind: 'clean' };
          if (deletedStr) return { title: `Cleaned ${deletedStr}`, subtitle: 'Completed', when, whenAt, kind: 'clean' };
          if (freed) return { title: `Cleaned ${freed}`, subtitle: 'Completed', when, whenAt, kind: 'clean' };
          return { title: 'Clean Operation', subtitle: 'Completed', when, whenAt, kind: 'clean' };
        }
        if (isScan) {
          const rawCategories = outcome.categoriesCount;
          const categoryCount = rawCategories != null ? Number(rawCategories) : null;
          const detail = categoryCount && categoryCount > 0 ? ` · ${categoryCount} ${categoryCount === 1 ? 'category' : 'categories'}` : '';
          return { title: `System Scan${detail}`, subtitle: 'Completed', when, whenAt, kind: 'scan' };
        }
      }
    } catch { /* fall through */ }
  }
  const lower = meta.name.toLowerCase();
  if (lower.includes('clean')) return { title: 'Clean Operation', subtitle: 'Completed', when, whenAt, kind: 'clean' };
  if (lower.includes('scan')) return { title: 'System Scan', subtitle: 'Completed', when, whenAt, kind: 'scan' };
  return { title: meta.name.replace(/\.jsonl$/i, ''), subtitle: 'Completed', when, whenAt, kind: 'all' };
}

function deriveActivitySummary(content: string | null): { primaryLabel: string; primaryValue: string; eligible: string; skipped: string; outcome: string; mode: string | null } {
  if (!content) return { primaryLabel: 'Space Reclaimed', primaryValue: '—', eligible: '—', skipped: '—', outcome: '—', mode: null };
  try {
    const lines = content.split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l) as Record<string, unknown>);
    const outcome = [...lines].reverse().find((o) => o.record === 'outcome') ?? [...lines].reverse()[0];
    const intent = lines.find((o) => o.record === 'intent') ?? lines[0];
    const isClean = !!(outcome?.deletedCount != null || outcome?.bytesFreed != null || outcome?.event === 'clean' || intent?.action === 'clean');
    const isScan = !isClean && (outcome?.event === 'scan' || intent?.action === 'scan' || outcome?.categoriesCount != null || (outcome?.totalBytes != null && outcome?.deletedCount == null));
    let primaryLabel = 'Space Reclaimed';
    let primaryValue = '—';
    if (isClean) {
      primaryLabel = 'Space Reclaimed';
      if (typeof outcome?.bytesFreed === 'string' || typeof outcome?.bytesFreed === 'number') primaryValue = formatBytes(String(outcome.bytesFreed));
    } else if (isScan) {
      primaryLabel = 'Space Found';
      if (typeof outcome?.totalBytes === 'string' || typeof outcome?.totalBytes === 'number') primaryValue = formatBytes(String(outcome.totalBytes));
    }

    const rows = parseAuditRows(content, null);
    const eligibleCategoryRows = rows.filter((r) => r.kind === 'Category' && r.outcomeTone === 'eligible');
    const cleanedCategoryRows = rows.filter((r) => r.kind === 'Category' && r.outcomeTone === 'cleaned');
    const eligibleOrCleanedCategories = (isClean ? cleanedCategoryRows.length : eligibleCategoryRows.length) || (outcome?.categoriesCount != null ? Number(outcome.categoriesCount) : (intent?.categoriesCount != null ? Number(intent.categoriesCount) : 0));
    const eligibleOrCleanedFiles = isClean
      ? Number(outcome?.deletedCount ?? rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'cleaned').length ?? 0)
      : Math.max(0, rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'eligible').length || (Number(outcome?.items ?? 0) - rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped').length));
    const skippedFileRows = rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
    const skippedCategories = new Set(
      skippedFileRows.map((r) => (r.sublabel || r.category || '').split('·')[0].trim()).filter(Boolean),
    ).size || (skippedFileRows.length > 0 ? skippedFileRows.length : 0);

    const eligible = `${eligibleOrCleanedFiles} ${eligibleOrCleanedFiles === 1 ? 'file' : 'files'} · ${eligibleOrCleanedCategories} ${eligibleOrCleanedCategories === 1 ? 'category' : 'categories'}`;
    const skipped = `${skippedFileRows.length} ${skippedFileRows.length === 1 ? 'file' : 'files'} · ${skippedCategories} ${skippedCategories === 1 ? 'category' : 'categories'}`;
    const status = String(outcome?.status ?? 'completed');
    return { primaryLabel, primaryValue, eligible, skipped, outcome: status === 'completed' ? 'Completed' : status, mode: isClean ? 'Permanent Delete' : null };
  } catch {
    return { primaryLabel: 'Space Reclaimed', primaryValue: '—', eligible: '—', skipped: '—', outcome: 'Recorded', mode: null };
  }
}
export function LogsView({ client, focusLogId, onFocusConsumed, onScanAgain }: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void; onScanAgain?: () => void }) {
  const [logs, setLogs] = useState<LogMeta[]>([]);
  const [contentById, setContentById] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<{ id: string; content: string } | null>(null);
  const [mode, setMode] = useState<'structured' | 'raw'>('structured');
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [filter, setFilter] = useState<LogFilter>('all');
  const [outcomeFilter, setOutcomeFilter] = useState<OutcomeFilter>('all');
  const [query, setQuery] = useState('');
  const [expandedSkippedGroups, setExpandedSkippedGroups] = useState<Set<string>>(new Set());

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
  const visibleAuditRows = useMemo(() => {
    if (outcomeFilter === 'all') return auditRows;
    if (outcomeFilter === 'eligible') return auditRows.filter((r) => r.outcomeTone === 'eligible');
    return auditRows.filter((r) => r.outcomeTone === 'skipped');
  }, [auditRows, outcomeFilter]);
  useEffect(() => { setOutcomeFilter('all'); setExpandedSkippedGroups(new Set()); }, [selected?.id]);
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
          <span className="header-subtitle num">{pluralize(logs.length, 'activity', 'activities')}</span>
        </div>
      </header>

      {error && <div className="notice error" role="alert">{error}</div>}

      <section className="logs-layout" aria-label="Activity history">
        <div className="logs-pane logs-pane-left">
          <div className="logs-pane-header logs-pane-header-stack">
            <h2 className="pane-section-title">Recent Activity</h2>
            <div className="logs-filter-bar">
              <div className="segmented-toggle scan-status-segmented" role="group" aria-label="Activity filters">
                <button type="button" className={`seg-btn ${filter === 'all' ? 'active' : ''}`} aria-pressed={filter === 'all'} onClick={() => setFilter('all')}>All</button>
                <button type="button" className={`seg-btn ${filter === 'scan' ? 'active' : ''}`} aria-pressed={filter === 'scan'} onClick={() => setFilter('scan')}>Scans</button>
                <button type="button" className={`seg-btn ${filter === 'clean' ? 'active' : ''}`} aria-pressed={filter === 'clean'} onClick={() => setFilter('clean')}>Cleanups</button>
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
                        <span className="log-secondary"><span>{sum.when}</span><span className="log-status-check" aria-label={sum.subtitle}><CheckIcon /> <span>{sum.subtitle}</span></span></span>
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
                {selectedSummary?.kind === 'scan' && onScanAgain && (
                  <button type="button" className="btn small secondary-action" onClick={onScanAgain} aria-label="Scan Again" title="Scan Again"><span>Scan Again</span></button>
                )}
              </div>
            )}
          </div>
          <div className="logs-pane-body log-details">
            {selected ? (
              <>
                <div aria-label="Structured log summary" className="log-summary-head">
                  {selectedSummary && <div className="log-summary-title">{selectedSummary.title}</div>}
                  {selectedSummary && (
                    <div className="muted log-summary-meta">
                      <span>{selectedSummary.whenAt}{activitySummary.mode ? <> · <span className="log-mode-badge">Mode: {activitySummary.mode}</span></> : null} · {selectedSummary.subtitle}</span>
                    </div>
                  )}
                </div>
                <div className="activity-summary-strip activity-summary-cards" aria-label="Activity summary">
                  <div className="activity-summary-cell">
                    <span className="activity-summary-label">{activitySummary.primaryLabel === 'Space Found' ? 'SPACE FOUND' : 'SPACE RECLAIMED'}</span>
                    <span className="activity-summary-value num">{activitySummary.primaryValue}</span>
                  </div>
                  <div className="activity-summary-cell">
                    <span className="activity-summary-label">{activitySummary.primaryLabel === 'Space Found' ? 'SAFE' : 'CLEANED'}</span>
                    <span className="activity-summary-value num">{activitySummary.eligible}</span>
                  </div>
                  <div className="activity-summary-cell">
                    <span className="activity-summary-label">SKIPPED</span>
                    <span className="activity-summary-value num">{activitySummary.skipped}</span>
                  </div>
                  <div className="activity-summary-cell">
                    <span className="activity-summary-label">OUTCOME</span>
                    <span className={`activity-summary-value activity-summary-outcome ${/completed/i.test(activitySummary.outcome) ? 'is-complete' : ''}`}>
                      {/completed/i.test(activitySummary.outcome) && <CheckIcon aria-hidden="true" />}
                      <span>{activitySummary.outcome}</span>
                    </span>
                  </div>
                </div>
                {mode === 'structured' ? (
                  <div className="log-structured audit-table-wrap logs-detail-fill">
                    {auditRows.length > 0 && (
                      <div className="logs-outcome-filter" role="group" aria-label="Outcome filters">
                        <div className="logs-outcome-inline-filters" role="group" aria-label="Outcome filters">
                          <button type="button" className={`logs-outcome-link ${outcomeFilter === 'all' ? 'active' : ''}`} aria-pressed={outcomeFilter === 'all'} onClick={() => setOutcomeFilter('all')}>All</button>
                          <button type="button" className={`logs-outcome-link ${outcomeFilter === 'eligible' ? 'active' : ''}`} aria-pressed={outcomeFilter === 'eligible'} onClick={() => setOutcomeFilter('eligible')}>Safe</button>
                          <button type="button" className={`logs-outcome-link ${outcomeFilter === 'skipped' ? 'active' : ''}`} aria-pressed={outcomeFilter === 'skipped'} onClick={() => setOutcomeFilter('skipped')}>Skipped</button>
                        </div>
                      </div>
                    )}
                    {auditRows.length === 0 ? <p className="muted">No entries.</p> : visibleAuditRows.length === 0 ? <p className="muted">No entries for this filter.</p> : (
                      (() => {
                        const categoryRows = visibleAuditRows.filter((r) => r.kind === 'Category');
                        const skippedRows = visibleAuditRows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
                        const otherRows = visibleAuditRows.filter((r) => r.kind === 'File' && r.outcomeTone !== 'skipped');
                        const renderRow = (r: AuditRow, idx: number, indented = false) => (
                          <tr key={`${idx}-${r.location}-${r.outcome}-${indented ? 's' : 'c'}`} className={`${r.kind === 'Category' ? 'audit-row-category' : 'audit-row-file'} ${indented ? 'audit-row-indented' : ''}`}>
                            <td className="audit-category">
                              <div className={`audit-category-cell ${indented ? 'audit-category-cell-indented' : ''}`}>
                                <span className="audit-category-main">{r.category}</span>
                                {r.sublabel ? <span className="audit-kind muted">{r.sublabel}</span> : null}
                              </div>
                            </td>
                            <td className="audit-location mono" title={r.location}>{String(r.location || '')}</td>
                            <td className="audit-size num">{r.size}</td>
                            <td className="audit-outcome"><span className={`badge outcome-pill outcome-${r.outcomeTone}${r.outcome.toLowerCase().includes('unknown') || r.outcome.toLowerCase().includes('unverified') ? ' badge-unknown' : ''}`}>{r.outcome}</span></td>
                          </tr>
                        );
                        return (
                          <table className="table audit-log-table logs-detail-table logs-detail-table-grouped">
                            <colgroup>
                              <col className="logs-col-category" />
                              <col className="logs-col-location" />
                              <col className="logs-col-size" />
                              <col className="logs-col-outcome" />
                            </colgroup>
                            <thead><tr><th scope="col">Category / Target</th><th scope="col">Location</th><th scope="col" className="num logs-th-size">Size</th><th scope="col" className="logs-th-outcome">Outcome</th></tr></thead>
                            <tbody>
                              {categoryRows.length > 0 && (
                                <tr className="audit-section-row"><td colSpan={4}>{(selectedSummary?.kind === 'scan' || activitySummary.primaryLabel === 'Space Found') ? 'SAFE CATEGORIES' : 'CLEANED CATEGORIES'}</td></tr>
                              )}
                              {categoryRows.map((r, i) => renderRow(r, i, false))}
                              {otherRows.map((r, i) => renderRow(r, i + 1000, false))}
                              {skippedRows.length > 0 && (
                                <tr className="audit-section-row"><td colSpan={4}>SKIPPED FILES</td></tr>
                              )}
                              {(() => {
                                const grouped = new Map<string, { key: string; label: string; reason: string; rows: AuditRow[] }>();
                                for (const r of skippedRows) {
                                  const parent = r.sublabel || r.category || 'Skipped';
                                  const reason = r.outcome.replace(/^Skipped\s*·?\s*/i, '').trim() || 'Skipped';
                                  const key = `${parent}::${reason}`;
                                  const label = (parent === 'Custom Path (Unknown)' || parent.startsWith('Custom Path') || parent.includes('CustomCleanup'))
                                    ? 'CustomCleanup'
                                    : parent;
                                  const g = grouped.get(key) ?? { key, label, reason, rows: [] };
                                  g.rows.push(r);
                                  grouped.set(key, g);
                                }
                                return Array.from(grouped.values()).map((g, gi) => {
                                  const expanded = expandedSkippedGroups.has(g.key);
                                  const displayReason = g.reason || 'Skipped';
                                  return (
                                    <Fragment key={`sg-${g.key}`}>
                                      <tr className="audit-row-category audit-skipped-group-row">
                                        <td className="audit-category" colSpan={2}>
                                          <button type="button" className="audit-skipped-group-toggle" onClick={() => setExpandedSkippedGroups((prev) => { const next = new Set(prev); if (next.has(g.key)) next.delete(g.key); else next.add(g.key); return next; })} aria-expanded={expanded}>
                                            <span className="audit-skipped-chevron" aria-hidden="true">
                                              <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true" focusable="false">
                                                <path d="M6 4.5 10 8l-4 3.5V4.5z" />
                                              </svg>
                                            </span>
                                            <span className="audit-category-main">{g.label}</span>
                                            {g.label === 'CustomCleanup' && <span className="scan-custom-path-tag muted">Custom Path</span>}
                                            <span className="audit-kind muted">· {g.rows.length} skipped ({displayReason})</span>
                                          </button>
                                        </td>
                                        <td className="audit-size num">{
                                          (() => {
                                            const totalBytes = g.rows.reduce((sum, r) => {
                                              const m = String(r.size || '').match(/^([\d.]+)\s*(B|KB|MB|GB|TB)$/i);
                                              if (!m) return sum;
                                              const val = parseFloat(m[1]);
                                              const unit = m[2].toUpperCase();
                                              const mult = unit === 'B' ? 1 : unit === 'KB' ? 1024 : unit === 'MB' ? 1024**2 : unit === 'GB' ? 1024**3 : 1024**4;
                                              return sum + (Number.isFinite(val) ? val * mult : 0);
                                            }, 0);
                                            return totalBytes > 0 ? formatBytes(String(Math.round(totalBytes))) : '—';
                                          })()
                                        }</td>
                                        <td className="audit-outcome"><span className="badge outcome-pill outcome-skipped">Skipped</span></td>
                                      </tr>
                                      {expanded && g.rows.map((r, i) => renderRow(r, 2000 + gi * 100 + i, true))}
                                    </Fragment>
                                  );
                                });
                              })()}
                            </tbody>
                          </table>
                        );
                      })()
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
