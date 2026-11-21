import { useCallback, useEffect, useMemo, useRef } from 'react';
import { formatContractError, type ScanResults, type SweeperClient } from '../../api/client';
import type { ContractError } from '../../contract/bindings';
import { SESSION_TTL_MS } from '../cleanShared';
import { filterScanItems, sortScanItems, visibleBaseTargets } from './scanHelpers';
import { autoSelectSafe, defaultCollapsed, inspectById, primaryScanItem, progressFromEvent, scanTiers } from './scanWorkflowActions';
import type { useScanState } from './useScanState';

export type ScanState = ReturnType<typeof useScanState>;
export type ScanToolbar = { query: string; sort: 'size' | 'name'; filter: 'all' | 'eligible' | 'skipped' };

function timerActive(scanStartedAt: number | null, sessionId: string | null): boolean {
  if (scanStartedAt == null) return false;
  return sessionId != null;
}

export function useScanTimer(args: { scanStartedAt: number | null; sessionId: string | null; setTick: (fn: (x: number) => number) => void }) {
  const { scanStartedAt, sessionId, setTick } = args;
  useEffect(() => {
    if (!timerActive(scanStartedAt, sessionId)) return;
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, [scanStartedAt, sessionId, setTick]);
}

export function useInvalidate(state: ScanState) {
  const previewRevisionRef = state.previewRevisionRef;
  const activePreviewRequestRef = state.activePreviewRequestRef;
  const setPreviewLoading = state.setPreviewLoading;
  return useCallback(() => {
    previewRevisionRef.current += 1;
    activePreviewRequestRef.current += 1;
    setPreviewLoading(false);
  }, [activePreviewRequestRef, previewRevisionRef, setPreviewLoading]);
}

function clearSelectionState(state: ScanState) {
  state.setSessionId(null);
  state.setScanStartedAt(null);
  state.setResults(null);
  state.setSelectedIds(new Set());
  state.setCollapsedTargets(new Set());
  state.setShowAllGroups(new Set());
  state.setScanSettingsVersion(null);
}

function clearPreviewState(state: ScanState) {
  state.setPreview(null);
  state.setConfirmText('');
  // A new Scan / reset advances the Preview request ID via invalidate(), so the
  // in-flight request can no longer clear its own determinate progress in its
  // finally block. Clear it here or "Preparing preview… N / N" stays visible.
  state.setPreviewProgress(null);
  state.setCleanProgress(null);
  state.setScanProgress(null);
  state.setScanStatus('');
  state.setRunningSessionId(null);
  state.setFocus(null);
  state.setInspected(null);
}

export function clearForReset(state: ScanState, invalidate: () => void) {
  invalidate();
  state.inspectReqIdRef.current += 1;
  state.setPreviewLoading(false);
  clearSelectionState(state);
  clearPreviewState(state);
}

export function useReset(state: ScanState, invalidate: () => void) {
  const setInfo = state.setInfo;
  return useCallback((message?: string) => {
    clearForReset(state, invalidate);
    if (message) setInfo(message);
  }, [invalidate, setInfo, state]);
}

export function useInspect(client: SweeperClient, state: ScanState) {
  const { inspectReqIdRef, setInspected } = state;
  return useCallback(async (itemId: string) => {
    const reqId = inspectReqIdRef.current + 1;
    inspectReqIdRef.current = reqId;
    await inspectById({ client, itemId, reqId, ref: inspectReqIdRef, setInspected: setInspected as never });
  }, [client, inspectReqIdRef, setInspected]);
}

export function prepareScanState(state: ScanState, invalidate: () => void) {
  invalidate();
  state.inspectReqIdRef.current += 1;
  state.setPreviewLoading(false);
  state.setError(null);
  state.setInfo(null);
  clearPreviewState(state);
  state.setSelectedIds(new Set());
  state.setCollapsedTargets(new Set());
  state.setShowAllGroups(new Set());
  state.setResults(null);
  state.setCleanSummary(null);
  state.setScanning(true);
  state.setScanStatus('Starting Scan\u2026');
  state.setScanProgress(null);
}

export function finalizeFocus(args: { data: ScanResults; state: ScanState; inspect: (id: string) => Promise<void> }) {
  const first = primaryScanItem({ data: args.data });
  if (first == null) {
    args.state.setFocus(null);
    return;
  }
  args.state.setFocus({ kind: 'item', itemId: first });
  void args.inspect(first);
}

export function isSessionInvalid(error: unknown): boolean {
  return (error as ContractError | undefined)?.code === 'sessionInvalid';
}

export function isScanExpired(scanStartedAt: number | null): boolean {
  if (scanStartedAt == null) return false;
  return Date.now() - scanStartedAt > SESSION_TTL_MS;
}

function filterGroup(args: { target: ScanResults['targets'][number]; toolbar: ScanToolbar; allow: boolean }) {
  const filtered = filterScanItems({ items: args.target.items, target: args.target, toolbar: args.toolbar, allowDeleteUnknown: args.allow });
  return sortScanItems({ items: filtered, target: args.target, sort: args.toolbar.sort });
}

function isQueryFiltered(toolbar: ScanToolbar): boolean {
  return toolbar.query.trim().length > 0;
}

function keepGroup(args: { items: unknown[]; toolbar: ScanToolbar }): boolean {
  if (args.items.length > 0) return true;
  if (isQueryFiltered(args.toolbar)) return false;
  return args.toolbar.filter === 'all';
}

export function useVisibleTargets(args: { results: ScanResults | null; toolbar: ScanToolbar; allow: boolean }) {
  const bases = useMemo(() => visibleBaseTargets({ targets: args.results?.targets ?? [] }), [args.results]);
  return useMemo(() => {
    return bases.map((target) => ({ target, items: filterGroup({ target, toolbar: args.toolbar, allow: args.allow }) }))
      .filter((group) => keepGroup({ items: group.items, toolbar: args.toolbar }));
  }, [args.allow, args.toolbar, bases]);
}

function hasTierChange(prev: { tier1: boolean; tier2: boolean }, next: { tier1: boolean; tier2: boolean }): boolean {
  if (prev.tier1 !== next.tier1) return true;
  return prev.tier2 !== next.tier2;
}

export function useTierRerun(args: { tier1: boolean; tier2: boolean }) {
  const { tier1, tier2 } = args;
  const prev = useRef({ tier1, tier2 });
  useEffect(() => {
    prev.current = { tier1, tier2 };
  }, [tier1, tier2]);
}

function runIsCurrent(ref: { current: number }, runId: number): boolean {
  return ref.current === runId;
}

export function isScanCancellation(error: unknown): boolean {
  // The typed variant's `code` is the discriminator (#94); never match a
  // free-text English reason (the core `ScanError` keeps its own message).
  const contractError = error as ContractError | undefined;
  return contractError?.code === 'scanCancelled';
}

export function applyProgress(args: { event: unknown; runId: number; ref: { current: number }; state: ScanState }) {
  if (!runIsCurrent(args.ref, args.runId)) return;
  const event = args.event as { kind: string; session_id?: string };
  // Progress events carry the session id allocated before the scan runs. That
  // is the only id Stop needs; no path is submitted (#94).
  if (event.session_id) args.state.setRunningSessionId(event.session_id);
  const progress = progressFromEvent(event);
  if (!progress) return;
  args.state.setScanStatus(progress.status);
  if (progress.progress) args.state.setScanProgress(progress.progress);
}

export function scanTiersArg(args: { tier1: boolean; tier2: boolean; enableTier3: boolean }) {
  return scanTiers({ tier1: args.tier1, tier2: args.tier2, enableTier3: args.enableTier3 });
}

export function finishScanRun(args: { ref: { current: number }; runId: number; state: ScanState }) {
  if (!runIsCurrent(args.ref, args.runId)) return;
  args.ref.current = 0;
  args.state.setScanning(false);
}

export function handleScanError(args: { error: unknown; ref: { current: number }; runId: number; state: ScanState; reset: (m?: string) => void }) {
  if (!runIsCurrent(args.ref, args.runId)) return;
  // A cancelled Scan ends with an Internal error carrying a stable reason.
  // Surface it as a cancelled state, not as a generic failure (#94).
  if (isScanCancellation(args.error)) {
    args.state.setScanStatus('Scan cancelled.');
    args.state.setScanProgress(null);
    return;
  }
  args.state.setError(formatContractError(args.error));
  if (isSessionInvalid(args.error)) args.reset();
}

export function baseView(state: ScanState) {
  return {
    tier1: state.tier1, setTier1: state.setTier1, tier2: state.tier2, setTier2: state.setTier2,
    scanning: state.scanning, scanStatus: state.scanStatus, scanProgress: state.scanProgress,
    sessionId: state.sessionId, results: state.results, selectedIds: state.selectedIds, setSelectedIds: state.setSelectedIds,
    collapsedTargets: state.collapsedTargets, setCollapsedTargets: state.setCollapsedTargets, toolbar: state.toolbar, setToolbar: state.setToolbar,
    focus: state.focus, setFocus: state.setFocus, inspected: state.inspected, preview: state.preview,
    previewLoading: state.previewLoading, setPreviewLoading: state.setPreviewLoading, confirmText: state.confirmText, setConfirmText: state.setConfirmText,
    cleaning: state.cleaning, setCleaning: state.setCleaning, cleanProgress: state.cleanProgress, cleanSummary: state.cleanSummary,
    setCleanSummary: state.setCleanSummary, setCleanProgress: state.setCleanProgress, error: state.error, setError: state.setError,
    info: state.info, setInfo: state.setInfo, relaunching: state.relaunching, setRelaunching: state.setRelaunching,
    relaunchInfo: state.relaunchInfo, setRelaunchInfo: state.setRelaunchInfo, relaunchError: state.relaunchError, setRelaunchError: state.setRelaunchError,
    showAllGroups: state.showAllGroups, setShowAllGroups: state.setShowAllGroups, scanSettingsVersion: state.scanSettingsVersion,
  };
}

export function applyScanData(args: { state: ScanState; data: ScanResults; settingsVersion?: number; allow: boolean; invalidate: () => void; inspect: (id: string) => Promise<void> }) {
  args.state.setResults(args.data);
  args.state.setScanSettingsVersion(args.settingsVersion ?? 0);
  args.invalidate();
  args.state.setSelectedIds(autoSelectSafe({ data: args.data, allowDeleteUnknown: args.allow }));
  args.state.setCollapsedTargets(defaultCollapsed({ baseVisible: visibleBaseTargets({ targets: args.data.targets }) }));
  finalizeFocus({ data: args.data, state: args.state, inspect: args.inspect });
  args.state.setScanStatus('');
  args.state.setScanProgress(null);
}