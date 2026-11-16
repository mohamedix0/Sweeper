import { useCallback, useEffect, useMemo, useRef } from 'react';
import { formatContractError, type ScanResults, type Settings, type SweeperClient } from '../../api/client';
import type { ContractError } from '../../contract/bindings';
import { SESSION_TTL_MS } from '../cleanShared';
import { filterScanItems, sortScanItems, visibleBaseTargets as getVisibleBaseTargets } from './scanHelpers';
import { autoSelectSafe, defaultCollapsed, inspectById, primaryScanItem, progressFromEvent, scanTiers } from './scanWorkflowActions';
import { useScanState } from './useScanState';

function useScanTimer(s: ReturnType<typeof useScanState>) {
  useEffect(() => {
    if (!s.scanStartedAt || !s.sessionId) return;
    const id = setInterval(() => s.setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, [s.scanStartedAt, s.sessionId, s.setTick]);
}

function applyProgress(ev: unknown, runId: number, ref: { current: number }, s: ReturnType<typeof useScanState>) {
  if (ref.current !== runId) return;
  const p = progressFromEvent(ev as { kind: string });
  if (!p) return;
  s.setScanStatus(p.status);
  if (p.progress) s.setScanProgress(p.progress);
}

function buildReturn<T, B, V>(args: { s: ReturnType<typeof useScanState>; settings: Settings | null; settingsVersion?: number; allow: boolean; targets: T[]; visibleBaseTargets: B[]; visibleTargets: V[]; expired: boolean; invalidate: () => void; inspect: (id: string) => Promise<void>; reset: (m?: string) => void; runScan: () => Promise<void> }) {
  const { s } = args;
  return {
    tier1: s.tier1, setTier1: s.setTier1, tier2: s.tier2, setTier2: s.setTier2, scanning: s.scanning, scanStatus: s.scanStatus, scanProgress: s.scanProgress, sessionId: s.sessionId, results: s.results, selectedIds: s.selectedIds, setSelectedIds: s.setSelectedIds, collapsedTargets: s.collapsedTargets, setCollapsedTargets: s.setCollapsedTargets, toolbar: s.toolbar, setToolbar: s.setToolbar, focus: s.focus, setFocus: s.setFocus, inspected: s.inspected, preview: s.preview, previewLoading: s.previewLoading, setPreviewLoading: s.setPreviewLoading, confirmText: s.confirmText, setConfirmText: s.setConfirmText, cleaning: s.cleaning, setCleaning: s.setCleaning, cleanProgress: s.cleanProgress, cleanSummary: s.cleanSummary, setCleanSummary: s.setCleanSummary, setCleanProgress: s.setCleanProgress, error: s.error, setError: s.setError, info: s.info, setInfo: s.setInfo, relaunching: s.relaunching, setRelaunching: s.setRelaunching, relaunchInfo: s.relaunchInfo, setRelaunchInfo: s.setRelaunchInfo, relaunchError: s.relaunchError, setRelaunchError: s.setRelaunchError, showAllGroups: s.showAllGroups, setShowAllGroups: s.setShowAllGroups, scanSettingsVersion: s.scanSettingsVersion, settings: args.settings, settingsVersion: args.settingsVersion, allowDeleteUnknown: args.allow, effectiveMinAgeHours: s.results?.minAgeHours ?? args.settings?.minAgeHours, targets: args.targets, visibleBaseTargets: args.visibleBaseTargets, visibleTargets: args.visibleTargets, isSessionExpired: args.expired,
    invalidatePreviewRequest: args.invalidate, inspectReqIdRef: s.inspectReqIdRef, activePreviewRequestRef: s.activePreviewRequestRef, previewRevisionRef: s.previewRevisionRef, inspectItem: args.inspect, resetForInvalidSession: args.reset, runScan: args.runScan, setPreview: s.setPreview, setResults: s.setResults,
  };
}

function useTierRerun(args: { tier1: boolean; tier2: boolean; ready: boolean; busy: boolean; runScan: () => Promise<void> }) {
  const prev = useRef({ tier1: args.tier1, tier2: args.tier2 });
  useEffect(() => {
    const changed = prev.current.tier1 !== args.tier1 || prev.current.tier2 !== args.tier2;
    prev.current = { tier1: args.tier1, tier2: args.tier2 };
    if (changed && args.ready && !args.busy) void args.runScan();
  }, [args.tier1, args.tier2, args.ready, args.busy, args.runScan]);
}

type ScanToolbar = { query: string; sort: 'size' | 'name'; filter: 'all' | 'eligible' | 'skipped' };

function useVisibleTargets(results: ScanResults | null, toolbar: ScanToolbar, allow: boolean) {
  const bases = useMemo(() => getVisibleBaseTargets(results?.targets ?? []), [results]);
  return useMemo(() => bases.map((t) => ({ target: t, items: sortScanItems({ items: filterScanItems({ items: t.items, target: t, toolbar, allowDeleteUnknown: allow }), target: t, sort: toolbar.sort }) })).filter((g) => g.items.length > 0 || (!toolbar.query.trim() && toolbar.filter === 'all')), [bases, toolbar, allow]);
}

export function useScanWorkflow(args: { client: SweeperClient; settings: Settings | null; settingsVersion?: number }) {
  const { client, settings, settingsVersion } = args;
  const s = useScanState();
  const allow = settings?.allowDeleteUnknown ?? false;
  const targets = useMemo(() => s.results?.targets ?? [], [s.results]);
  const visibleBaseTargets = useMemo(() => getVisibleBaseTargets(targets), [targets]);
  useScanTimer(s);
  const expired = !!s.scanStartedAt && Date.now() - s.scanStartedAt > SESSION_TTL_MS;

  const invalidate = useCallback(() => { s.previewRevisionRef.current += 1; s.activePreviewRequestRef.current += 1; s.setPreviewLoading(false); }, [s.activePreviewRequestRef, s.previewRevisionRef, s.setPreviewLoading]);
  const reset = useCallback((message?: string) => {
    invalidate(); s.inspectReqIdRef.current += 1; s.setPreviewLoading(false);
    s.setSessionId(null); s.setScanStartedAt(null); s.setResults(null); s.setSelectedIds(new Set());
    s.setCollapsedTargets(new Set()); s.setShowAllGroups(new Set()); s.setScanSettingsVersion(null);
    s.setPreview(null); s.setConfirmText(''); s.setCleanProgress(null); s.setScanProgress(null);
    s.setScanStatus(''); s.setFocus(null); s.setInspected(null); if (message) s.setInfo(message);
  }, [invalidate, s]);

  const inspect = useCallback(async (itemId: string) => {
    const reqId = s.inspectReqIdRef.current + 1; s.inspectReqIdRef.current = reqId;
    await inspectById({ client, itemId, reqId, ref: s.inspectReqIdRef, setInspected: s.setInspected as never });
  }, [client, s.inspectReqIdRef, s.setInspected]);

  const prepareScan = useCallback(() => {
    invalidate(); s.inspectReqIdRef.current += 1; s.setPreviewLoading(false);
    s.setError(null); s.setInfo(null); s.setPreview(null); s.setFocus(null); s.setInspected(null);
    s.setSelectedIds(new Set()); s.setCollapsedTargets(new Set()); s.setShowAllGroups(new Set());
    s.setResults(null); s.setCleanSummary(null); s.setCleanProgress(null);
    s.setScanning(true); s.setScanStatus('Starting Scan…'); s.setScanProgress(null);
  }, [invalidate, s]);

  const applyData = useCallback(async (sid: string, runId: number) => {
    s.setSessionId(sid); s.setScanStartedAt(Date.now());
    const data = await client.getScanResults(sid);
    if (s.activeScanRunRef.current !== runId) return;
    s.setResults(data); s.setScanSettingsVersion(settingsVersion ?? 0);
    invalidate(); s.setSelectedIds(autoSelectSafe(data, allow));
    s.setCollapsedTargets(defaultCollapsed(getVisibleBaseTargets(data.targets)));
    const first = primaryScanItem(data);
    if (first) { s.setFocus({ kind: 'item', itemId: first }); void inspect(first); } else s.setFocus(null);
    s.setScanStatus(''); s.setScanProgress(null);
  }, [allow, client, inspect, invalidate, s, settingsVersion]);

  const runScan = useCallback(async () => {
    if (s.activeScanRunRef.current !== 0) return;
    const runId = s.activeScanRunRef.current + 1; s.activeScanRunRef.current = runId;
    prepareScan();
    try {
      const { sessionId: sid } = await client.startScan(scanTiers(s.tier1, s.tier2), (ev) => applyProgress(ev, runId, s.activeScanRunRef, s));
      if (s.activeScanRunRef.current !== runId) return;
      await applyData(sid, runId);
    } catch (e) {
      if (s.activeScanRunRef.current !== runId) return;
      s.setError(formatContractError(e));
      if ((e as ContractError | undefined)?.code === 'sessionInvalid') reset();
    } finally { if (s.activeScanRunRef.current === runId) { s.activeScanRunRef.current = 0; s.setScanning(false); } }
  }, [applyData, client, prepareScan, reset, s]);

  useTierRerun({ tier1: s.tier1, tier2: s.tier2, ready: !!s.results, busy: s.scanning || s.cleaning, runScan });
  const visibleTargets = useVisibleTargets(s.results, s.toolbar, allow);
  return buildReturn({ s, settings, settingsVersion, allow, targets, visibleBaseTargets, visibleTargets, expired, invalidate, inspect, reset, runScan });
}
