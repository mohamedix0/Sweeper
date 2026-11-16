import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { formatContractError, type SweeperClient } from '../../api/client';
import { matchesLogFilter } from './LogEntryCard';
import type { LogFilter, LogMeta } from './logParsers';
import { readLogMap } from './useLogsSelectionHelpers';
import { contentForLog, useLogsDerived, useLogsUiState } from './useLogsSelectionUi';

type SelectedLog = { id: string; content: string };
type LogRequest = { seq: number; id: string };

function mergeLogMap(prev: Record<string, string>, next: Record<string, string>) {
  if (Object.keys(next).length === 0) return prev;
  return { ...prev, ...next };
}

function isLatest(args: { req: LogRequest; ref: React.MutableRefObject<LogRequest> }) {
  if (args.ref.current.seq !== args.req.seq) return false;
  return args.ref.current.id === args.req.id;
}

function claimRequest(args: { id: string; ref: React.MutableRefObject<LogRequest> }) {
  const seq = args.ref.current.seq + 1;
  args.ref.current = { seq, id: args.id };
  return args.ref.current;
}

function applyCachedLog(args: { log: SelectedLog; req: LogRequest; ref: React.MutableRefObject<LogRequest>; setSelected: (v: SelectedLog) => void; setContentById: React.Dispatch<React.SetStateAction<Record<string, string>>> }) {
  if (!isLatest({ req: args.req, ref: args.ref })) return;
  args.setSelected(args.log);
  args.setContentById((prev) => ({ ...prev, [args.log.id]: args.log.content }));
}

function applyLogError(args: { error: unknown; req: LogRequest; ref: React.MutableRefObject<LogRequest>; setError: (v: string | null) => void }) {
  if (!isLatest({ req: args.req, ref: args.ref })) return;
  args.setError(formatContractError(args.error));
}

async function readOwnedLog(args: { client: SweeperClient; id: string; requestRef: React.MutableRefObject<LogRequest>; setError: (v: string | null) => void; setSelected: (v: SelectedLog) => void; setContentById: React.Dispatch<React.SetStateAction<Record<string, string>>> }) {
  const req = claimRequest({ id: args.id, ref: args.requestRef });
  args.setError(null);
  try {
    const log = await args.client.readLog(args.id);
    applyCachedLog({ log, req, ref: args.requestRef, setSelected: args.setSelected, setContentById: args.setContentById });
  } catch (error) {
    applyLogError({ error, req, ref: args.requestRef, setError: args.setError });
  }
}

async function refreshLogs(args: { client: SweeperClient; cancelled: () => boolean; setLogs: (v: LogMeta[]) => void; setContentById: React.Dispatch<React.SetStateAction<Record<string, string>>>; setError: (v: string | null) => void }) {
  try {
    const next = await args.client.listLogs();
    if (args.cancelled()) return [] as LogMeta[];
    args.setLogs(next);
    void readLogMap(args.client, next).then((map) => {
      if (args.cancelled()) return;
      args.setContentById((p) => mergeLogMap(p, map));
    });
    return next;
  } catch (error) {
    if (!args.cancelled()) args.setError(formatContractError(error));
    return [] as LogMeta[];
  }
}

function useLogData(client: SweeperClient) {
  const [logs, setLogs] = useState<LogMeta[]>([]);
  const [contentById, setContentById] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<SelectedLog | null>(null);
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef<LogRequest>({ seq: 0, id: '' });
  const refresh = useCallback(() => refreshLogs({ client, cancelled: () => false, setLogs, setContentById, setError }), [client]);
  const clearSelected = useCallback(() => {
    requestRef.current = { seq: requestRef.current.seq + 1, id: '' };
    setSelected(null);
  }, []);
  const openLog = useCallback(async (id: string) => {
    await readOwnedLog({ client, id, requestRef, setError, setSelected, setContentById });
  }, [client]);
  return { logs, contentById, selected, error, setError, setLogs, setContentById, setSelected, clearSelected, requestRef, refresh, openLog };
}

function canAutoOpen(args: { focusLogId?: string | null; selected: SelectedLog | null; next: LogMeta[]; suppress: boolean }) {
  if (args.focusLogId) return false;
  if (args.selected) return false;
  if (args.suppress) return false;
  return args.next.length > 0;
}

function useAutoOpen(args: { client: SweeperClient; focusLogId?: string | null; data: ReturnType<typeof useLogData>; suppress: boolean }) {
  const { client, focusLogId, data, suppress } = args;
  const { selected, setLogs, setContentById, setError, requestRef, setSelected } = data;
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const next = await refreshLogs({ client, cancelled: () => cancelled, setLogs, setContentById, setError });
      if (cancelled) return;
      if (!canAutoOpen({ focusLogId, selected, next, suppress })) return;
      await readOwnedLog({ client, id: next[0].id, requestRef, setError, setSelected, setContentById });
    })();
    return () => { cancelled = true; };
  }, [client, focusLogId, requestRef, selected, setContentById, setError, setLogs, setSelected, suppress]);
}

function useFocusOpen(args: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void; data: ReturnType<typeof useLogData> }) {
  const { client, focusLogId, onFocusConsumed, data } = args;
  const focusConsumedRef = useRef(onFocusConsumed);
  focusConsumedRef.current = onFocusConsumed;
  const { setLogs, setContentById, setError, requestRef, setSelected } = data;
  useEffect(() => {
    if (!focusLogId) return;
    let cancelled = false;
    void (async () => {
      await refreshLogs({ client, cancelled: () => cancelled, setLogs, setContentById, setError });
      if (cancelled) return;
      await readOwnedLog({ client, id: focusLogId, requestRef, setError, setSelected, setContentById });
      if (cancelled) return;
      focusConsumedRef.current?.();
    })();
    return () => { cancelled = true; };
  }, [client, focusLogId, requestRef, setContentById, setError, setLogs, setSelected]);
}

function useLogActions(args: { data: ReturnType<typeof useLogData>; selectedMetaName: string | undefined; setCopied: (v: boolean) => void; setExpandedSkippedGroups: React.Dispatch<React.SetStateAction<Set<string>>> }) {
  const { selected } = args.data;
  async function copyLog() {
    if (!selected) return;
    try { await navigator.clipboard?.writeText(selected.content); args.setCopied(true); setTimeout(() => args.setCopied(false), 1500); } catch { /* ignore */ }
  }
  function exportLog() {
    if (!selected) return;
    const blob = new Blob([selected.content], { type: 'application/x-ndjson' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = args.selectedMetaName || `${selected.id}.jsonl`;
    a.click();
    URL.revokeObjectURL(url);
  }
  function toggleSkippedGroup(key: string) {
    args.setExpandedSkippedGroups((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; });
  }
  return { copyLog, exportLog, toggleSkippedGroup };
}

function useFilterSelectionSync(args: {
  filteredLogs: LogMeta[];
  selectedId: string | null;
  openLog: (id: string) => Promise<void>;
  clearSelected: () => void;
  focusLogId?: string | null;
}) {
  const { filteredLogs, selectedId, openLog, clearSelected, focusLogId } = args;
  useEffect(() => {
    if (focusLogId) return;
    if (filteredLogs.length === 0) {
      if (selectedId) clearSelected();
      return;
    }
    const inFilter = selectedId ? filteredLogs.some((l) => l.id === selectedId) : false;
    if (!inFilter) {
      void openLog(filteredLogs[0].id);
    }
  }, [clearSelected, filteredLogs, focusLogId, openLog, selectedId]);
}

function isSelectedInFilter(selectedId: string | undefined, filteredLogs: LogMeta[]): boolean {
  if (!selectedId) return false;
  return filteredLogs.some((l) => l.id === selectedId);
}

function resolveEffectiveSelected(selected: SelectedLog | null, filteredLogs: LogMeta[]) {
  if (!isSelectedInFilter(selected?.id, filteredLogs)) return null;
  return selected;
}

function useFilteredLogs(args: {
  logs: LogMeta[];
  contentById: Record<string, string>;
  selected: SelectedLog | null;
  filter: LogFilter;
  query: string;
}) {
  const { logs, contentById, selected, filter, query } = args;
  return useMemo(
    () =>
      logs.filter((l) =>
        matchesLogFilter({
          meta: l,
          content: contentForLog({ meta: l, selected, contentById }),
          filter,
          query,
        }),
      ),
    [logs, contentById, selected, filter, query],
  );
}

export function useLogsSelectionModel(args: {
  client: SweeperClient;
  focusLogId?: string | null;
  onFocusConsumed?: () => void;
}) {
  const { client, focusLogId, onFocusConsumed } = args;
  const data = useLogData(client);
  const ui = useLogsUiState(data.selected?.id);
  const query = ui.query.trim().toLowerCase();
  const filteredLogs = useFilteredLogs({
    logs: data.logs,
    contentById: data.contentById,
    selected: data.selected,
    filter: ui.filter,
    query,
  });
  const suppress = !focusLogId && !data.selected && filteredLogs.length === 0 && data.logs.length > 0;

  useAutoOpen({ client, focusLogId, data, suppress });
  useFocusOpen({ client, focusLogId, onFocusConsumed, data });
  useFilterSelectionSync({
    filteredLogs,
    selectedId: data.selected?.id ?? null,
    openLog: data.openLog,
    clearSelected: data.clearSelected,
    focusLogId,
  });

  const effectiveSelected = resolveEffectiveSelected(data.selected, filteredLogs);
  const derived = useLogsDerived({
    logs: data.logs,
    selected: effectiveSelected,
    outcomeFilter: ui.outcomeFilter,
  });
  const actions = useLogActions({
    data,
    selectedMetaName: derived.selectedMeta?.name,
    setCopied: ui.setCopied,
    setExpandedSkippedGroups: ui.setExpandedSkippedGroups,
  });

  return {
    logs: data.logs,
    selected: effectiveSelected,
    mode: ui.mode,
    setMode: ui.setMode,
    error: data.error,
    copied: ui.copied,
    filter: ui.filter,
    setFilter: ui.setFilter,
    outcomeFilter: ui.outcomeFilter,
    setOutcomeFilter: ui.setOutcomeFilter,
    query: ui.query,
    setQuery: ui.setQuery,
    selectedMeta: derived.selectedMeta,
    auditRows: derived.auditRows,
    visibleAuditRows: derived.visibleAuditRows,
    selectedSummary: derived.selectedSummary,
    activitySummary: derived.activitySummary,
    filteredLogs,
    contentById: data.contentById,
    expandedSkippedGroups: ui.expandedSkippedGroups,
    openLog: data.openLog,
    copyLog: actions.copyLog,
    exportLog: actions.exportLog,
    toggleSkippedGroup: actions.toggleSkippedGroup,
  };
}
