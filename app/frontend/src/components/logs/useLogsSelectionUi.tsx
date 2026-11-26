import { useMemo, useState } from 'react';
import type { AuditRow, LogFilter, LogMeta, OutcomeFilter } from './logParsers';
import { deriveActivitySummary, parseAuditRows, summarizeLog } from './logParsers';

type SelectedLog = { id: string; content: string };

function filterAuditRows(rows: AuditRow[], filter: OutcomeFilter): AuditRow[] {
  if (filter === 'all') return rows;
  if (filter === 'eligible') return rows.filter((r) => r.outcomeTone === 'eligible');
  return rows.filter((r) => r.outcomeTone === 'skipped');
}

function findMeta(logs: LogMeta[], selected: SelectedLog | null): LogMeta | null {
  if (!selected) return null;
  const found = logs.find((l) => l.id === selected.id);
  if (found) return found;
  return null;
}

export function contentForLog(args: { meta: LogMeta; selected: SelectedLog | null; contentById: Record<string, string> }): string | null {
  let c: string | null = args.contentById[args.meta.id] ?? null;
  if (!args.selected) return c;
  if (args.selected.id === args.meta.id) c = args.selected.content;
  return c;
}

export function useLogsUiState(selectedId: string | undefined) {
  const [mode, setMode] = useState<'structured' | 'raw'>('structured');
  const [copied, setCopied] = useState(false);
  const [filter, setFilter] = useState<LogFilter>('all');
  const [outcomeFilter, setOutcomeFilter] = useState<OutcomeFilter>('all');
  const [query, setQuery] = useState('');
  const [expandedSkippedGroups, setExpandedSkippedGroups] = useState<Set<string>>(new Set());
  useMemo(() => { setOutcomeFilter('all'); setExpandedSkippedGroups(new Set()); return selectedId; }, [selectedId]);
  return { mode, setMode, copied, setCopied, filter, setFilter, outcomeFilter, setOutcomeFilter, query, setQuery, expandedSkippedGroups, setExpandedSkippedGroups };
}

export function useLogsDerived(args: { logs: LogMeta[]; selected: SelectedLog | null; outcomeFilter: OutcomeFilter }) {
  const selectedMeta = useMemo(() => findMeta(args.logs, args.selected), [args.logs, args.selected]);
  const auditRows = useMemo(() => {
    if (!args.selected) return [];
    return parseAuditRows(args.selected.content);
  }, [args.selected]);
  const visibleAuditRows = useMemo(() => filterAuditRows(auditRows, args.outcomeFilter), [auditRows, args.outcomeFilter]);
  const selectedSummary = useMemo(() => {
    if (!selectedMeta) return null;
    let content: string | null = null;
    if (args.selected) content = args.selected.content;
    return summarizeLog(selectedMeta, content);
  }, [selectedMeta, args.selected]);
  const selectedContent = args.selected?.content ?? null;
  const activitySummary = useMemo(() => deriveActivitySummary(selectedContent), [selectedContent]);
  return { selectedMeta, auditRows, visibleAuditRows, selectedSummary, activitySummary };
}
