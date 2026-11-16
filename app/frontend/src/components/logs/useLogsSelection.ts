import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatContractError, type SweeperClient } from '../../api/client';
import { formatBytes } from '../../utils/format';
import { matchesLogFilter } from './LogEntryCard';
import { deriveActivitySummary, parseAuditRows, summarizeLog, type AuditRow, type LogFilter, type LogMeta, type OutcomeFilter } from './logParsers';

function sizeFromLabel(size: string): number {
  const m = String(size || '').match(/^([\d.]+)\s*(B|KB|MB|GB|TB)$/i);
  if (!m) return 0;
  const val = parseFloat(m[1]);
  const unit = m[2].toUpperCase();
  const mult = unit === 'B' ? 1 : unit === 'KB' ? 1024 : unit === 'MB' ? 1024 ** 2 : unit === 'GB' ? 1024 ** 3 : 1024 ** 4;
  return Number.isFinite(val) ? val * mult : 0;
}

export function groupSkippedRows(rows: AuditRow[]) {
  const grouped = new Map<string, { key: string; label: string; reason: string; rows: AuditRow[] }>();
  for (const r of rows) {
    const parent = r.sublabel || r.category || 'Skipped';
    const reason = r.outcome.replace(/^Skipped\s*·?\s*/i, '').trim() || 'Skipped';
    const key = `${parent}::${reason}`;
    const isCustom = parent === 'Custom Path (Unknown)' || parent.startsWith('Custom Path') || parent.includes('CustomCleanup');
    const label = isCustom ? 'CustomCleanup' : parent;
    const g = grouped.get(key) ?? { key, label, reason, rows: [] };
    g.rows.push(r);
    grouped.set(key, g);
  }
  return Array.from(grouped.values());
}

export function formatGroupBytes(rows: AuditRow[]): string {
  const total = rows.reduce((sum, r) => sum + sizeFromLabel(r.size), 0);
  return total > 0 ? formatBytes(String(Math.round(total))) : '—';
}

type SelectedLog = { id: string; content: string };

async function readLogMap(client: SweeperClient, items: LogMeta[]) {
  const entries = await Promise.all(items.slice(0, 15).map(async (item) => {
    try { return [item.id, (await client.readLog(item.id)).content] as const; } catch { return null; }
  }));
  const map: Record<string, string> = {};
  for (const entry of entries) if (entry) map[entry[0]] = entry[1];
  return map;
}

function filterAuditRows(rows: AuditRow[], filter: OutcomeFilter): AuditRow[] {
  if (filter === 'all') return rows;
  if (filter === 'eligible') return rows.filter((r) => r.outcomeTone === 'eligible');
  return rows.filter((r) => r.outcomeTone === 'skipped');
}

export function useLogsSelection(args: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void }) {
  const { client, focusLogId, onFocusConsumed } = args;
  const [logs, setLogs] = useState<LogMeta[]>([]);
  const [contentById, setContentById] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<SelectedLog | null>(null);
  const [mode, setMode] = useState<'structured' | 'raw'>('structured');
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [filter, setFilter] = useState<LogFilter>('all');
  const [outcomeFilter, setOutcomeFilter] = useState<OutcomeFilter>('all');
  const [query, setQuery] = useState('');
  const [expandedSkippedGroups, setExpandedSkippedGroups] = useState<Set<string>>(new Set());

  const cacheLog = useCallback((log: SelectedLog) => {
    setSelected(log);
    setContentById((prev) => ({ ...prev, [log.id]: log.content }));
  }, []);

  const refresh = useCallback(async (): Promise<LogMeta[]> => {
    try {
      const next = await client.listLogs();
      setLogs(next);
      void readLogMap(client, next).then((map) => { if (Object.keys(map).length) setContentById((p) => ({ ...p, ...map })); });
      return next;
    } catch (e) { setError(formatContractError(e)); return []; }
  }, [client]);

  const openLog = useCallback(async (id: string) => {
    setError(null);
    try { cacheLog(await client.readLog(id)); } catch (e) { setError(formatContractError(e)); }
  }, [cacheLog, client]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next = await refresh();
      if (cancelled || focusLogId || selected || next.length === 0) return;
      try { const log = await client.readLog(next[0].id); if (!cancelled) cacheLog(log); }
      catch (e) { if (!cancelled) setError(formatContractError(e)); }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);

  useEffect(() => {
    if (!focusLogId) return;
    let cancelled = false;
    (async () => {
      setError(null);
      try { await refresh(); const log = await client.readLog(focusLogId); if (!cancelled) cacheLog(log); }
      catch (e) { if (!cancelled) setError(formatContractError(e)); }
      finally { if (!cancelled) onFocusConsumed?.(); }
    })();
    return () => { cancelled = true; };
  }, [focusLogId, client, refresh, onFocusConsumed, cacheLog]);

  const selectedMeta = useMemo(() => (selected ? logs.find((l) => l.id === selected.id) ?? null : null), [logs, selected]);
  const auditRows = useMemo(() => (selected ? parseAuditRows(selected.content, selectedMeta) : []), [selected, selectedMeta]);
  const visibleAuditRows = useMemo(() => filterAuditRows(auditRows, outcomeFilter), [auditRows, outcomeFilter]);
  useEffect(() => { setOutcomeFilter('all'); setExpandedSkippedGroups(new Set()); }, [selected?.id]);
  const selectedSummary = useMemo(() => (selectedMeta ? summarizeLog(selectedMeta, selected?.content ?? null) : null), [selectedMeta, selected]);
  const activitySummary = useMemo(() => deriveActivitySummary(selected?.content ?? null), [selected]);
  const q = query.trim().toLowerCase();
  const filteredLogs = useMemo(() => logs.filter((l) => {
    const c = selected && selected.id === l.id ? selected.content : contentById[l.id] ?? null;
    return matchesLogFilter({ meta: l, content: c, filter, query: q });
  }), [logs, contentById, selected, filter, q]);

  async function copyLog() {
    if (!selected) return;
    try { await navigator.clipboard?.writeText(selected.content); setCopied(true); setTimeout(() => setCopied(false), 1500); } catch { /* ignore */ }
  }
  function exportLog() {
    if (!selected) return;
    const blob = new Blob([selected.content], { type: 'application/x-ndjson' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = selectedMeta?.name || `${selected.id}.jsonl`; a.click();
    URL.revokeObjectURL(url);
  }
  function toggleSkippedGroup(key: string) {
    setExpandedSkippedGroups((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; });
  }

  return {
    logs, selected, mode, setMode, error, copied, filter, setFilter, outcomeFilter, setOutcomeFilter, query, setQuery,
    selectedMeta, auditRows, visibleAuditRows, selectedSummary, activitySummary, filteredLogs, contentById, expandedSkippedGroups,
    openLog, copyLog, exportLog, toggleSkippedGroup,
  };
}
