import { useCallback, useEffect, useMemo, useRef } from 'react';
import type { ScanResults, Settings, SweeperClient } from '../../api/client';
import { visibleBaseTargets } from './scanHelpers';
import { useScanState } from './useScanState';
import { applyProgress, applyScanData, baseView, finishScanRun, handleScanError, isScanExpired, prepareScanState, scanTiersArg, STOPPING_SCAN_NOTICE, useInspect, useInvalidate, useReset, useScanTimer, useTierRerun, useVisibleTargets } from './useScanWorkflowRuntime';

interface WorkflowArgs { client: SweeperClient; settings: Settings | null; settingsVersion?: number }

function allowUnknown(settings: Settings | null): boolean {
  return settings?.allowDeleteUnknown ?? false;
}

function minAgeForView(args: { results: ScanResults | null; settings: Settings | null }) {
  if (args.results?.minAgeHours != null) return args.results.minAgeHours;
  return args.settings?.minAgeHours;
}

function runIsCurrent(args: { ref: { current: number }; runId: number }): boolean {
  return args.ref.current === args.runId;
}

function canStartRun(ref: { current: number }): boolean {
  return ref.current === 0;
}

function nextRunId(ref: { current: number }): number {
  const runId = ref.current + 1;
  ref.current = runId;
  return runId;
}

function hasActiveScope(args: { tier1: boolean; tier2: boolean; enableTier3: boolean }) {
  if (args.tier1) return true;
  if (args.tier2) return true;
  return args.enableTier3;
}

function tier3Enabled(settings: Settings | null): boolean {
  return settings?.enableTier3 ?? false;
}

function useScanStartAction(args: { client: SweeperClient; settingsVersion?: number; state: ReturnType<typeof useScanState>; allow: boolean; invalidate: () => void; reset: (m?: string) => void; inspect: (id: string) => Promise<void>; tier1: boolean; tier2: boolean; enableTier3: boolean }) {
  const { client, settingsVersion, state, allow, invalidate, reset, inspect, tier1, tier2, enableTier3 } = args;
  const { activeScanRunRef, cleaning } = state;
  const prepare = useCallback(() => prepareScanState(state, invalidate), [invalidate, state]);
  const applyData = useCallback(async (argsData: { sid: string; runId: number }) => {
    state.setSessionId(argsData.sid);
    state.setScanStartedAt(Date.now());
    const data = await client.getScanResults(argsData.sid);
    if (!runIsCurrent({ ref: activeScanRunRef, runId: argsData.runId })) return;
    applyScanData({ state, data, settingsVersion, allow, invalidate, inspect });
  }, [activeScanRunRef, allow, client, inspect, invalidate, settingsVersion, state]);
  const runScanBody = useCallback(async (argsRun: { runId: number; tiers: ReturnType<typeof scanTiersArg> }) => {
    const started = await client.startScan(argsRun.tiers, (event) => applyProgress({ event, runId: argsRun.runId, ref: activeScanRunRef, state }));
    if (!runIsCurrent({ ref: activeScanRunRef, runId: argsRun.runId })) return;
    await applyData({ sid: started.sessionId, runId: argsRun.runId });
  }, [activeScanRunRef, applyData, client, state]);
  return useCallback(async () => {
    if (!canStartRun(activeScanRunRef)) return;
    if (cleaning) return;
    const tiers = scanTiersArg({ tier1, tier2, enableTier3 });
    if (tiers.length === 0) return;
    const runId = nextRunId(activeScanRunRef);
    prepare();
    try {
      await runScanBody({ runId, tiers });
    } catch (error) {
      handleScanError({ error, ref: activeScanRunRef, runId, state, reset });
    } finally {
      finishScanRun({ ref: activeScanRunRef, runId, state });
    }
  }, [activeScanRunRef, cleaning, enableTier3, prepare, reset, runScanBody, state, tier1, tier2]);
}

function useStopScan(args: { client: SweeperClient; state: ReturnType<typeof useScanState> }) {
  const { client, state } = args;
  const pendingStopRef = useRef(false);
  const submitStop = useCallback(async (sid: string) => {
    try {
      await client.cancelScan(sid);
      state.setInfo(STOPPING_SCAN_NOTICE);
    } catch {
      state.setError('Failed to stop Scan.');
    }
  }, [client, state]);

  // Stop can be clicked while the Scan is running but before any progress event
  // supplies its session id. Retain that intent and submit it once the id
  // arrives, without losing the known-session path (#94).
  useEffect(() => {
    if (!state.scanning) {
      pendingStopRef.current = false;
      return;
    }
    const sid = state.runningSessionId;
    if (pendingStopRef.current && sid) {
      pendingStopRef.current = false;
      void submitStop(sid);
    }
  }, [state.runningSessionId, state.scanning, submitStop]);

  return useCallback(async () => {
    const sid = state.runningSessionId;
    if (!sid) {
      if (state.scanning) pendingStopRef.current = true;
      return;
    }
    if (!state.scanning) return;
    await submitStop(sid);
  }, [state, submitStop]);
}

function useScanActions(args: { client: SweeperClient; settings: Settings | null; settingsVersion?: number; state: ReturnType<typeof useScanState>; allow: boolean; invalidate: () => void; reset: (m?: string) => void; inspect: (id: string) => Promise<void> }) {
  const { client, settings, settingsVersion, state, allow, invalidate, reset, inspect } = args;
  const { tier1, tier2 } = state;
  const enableTier3 = tier3Enabled(settings);
  const runScan = useScanStartAction({ client, settingsVersion, state, allow, invalidate, reset, inspect, tier1, tier2, enableTier3 });
  const stopScan = useStopScan({ client, state });
  return { runScan, stopScan, hasActiveScanScope: hasActiveScope({ tier1, tier2, enableTier3 }) };
}

export function useScanWorkflow(args: WorkflowArgs) {
  const { client, settings, settingsVersion } = args;
  const state = useScanState();
  const { scanStartedAt, sessionId, setTick, results, toolbar, tier1, tier2 } = state;
  const allow = allowUnknown(settings);
  const targets = useMemo(() => results?.targets ?? [], [results]);
  const visibleBase = useMemo(() => visibleBaseTargets({ targets }), [targets]);
  useScanTimer({ scanStartedAt, sessionId, setTick });
  const expired = isScanExpired(scanStartedAt);
  const invalidate = useInvalidate(state);
  const reset = useReset(state, invalidate);
  const inspect = useInspect(client, state);
  const { runScan, stopScan, hasActiveScanScope } = useScanActions({ client, settings, settingsVersion, state, allow, invalidate, reset, inspect });
  useTierRerun({ tier1, tier2 });
  const visibleTargets = useVisibleTargets({ results, toolbar, allow });
  return {
    ...baseView(state), settings, settingsVersion, allowDeleteUnknown: allow, effectiveMinAgeHours: minAgeForView({ results, settings }),
    targets, visibleBaseTargets: visibleBase, visibleTargets, isSessionExpired: expired, hasActiveScanScope,
    invalidatePreviewRequest: invalidate, inspectReqIdRef: state.inspectReqIdRef, activePreviewRequestRef: state.activePreviewRequestRef,
    previewRevisionRef: state.previewRevisionRef, inspectItem: inspect, resetForInvalidSession: reset, runScan, stopScan,
    previewProgress: state.previewProgress, setPreviewProgress: state.setPreviewProgress,
    setPreview: state.setPreview, setResults: state.setResults,
  };
}
