import type { ContractError, SweeperClient } from '../client';
import { defaultSeedLogs } from './data';
import { buildScanAnalyzerApi } from './scanAnalyzerOps';
import { buildPreviewApi } from './previewOps';
import { buildCleanApi } from './cleanOps';
import { buildConfigApi, defaultSettings } from './configOps';
import { buildTestApi } from './testOps';
import type { AnalyzerSessionState, LogEntry, PreviewState, ScanSessionState } from './types';

export type { MockCtx } from './types';
export type MockController = ReturnType<typeof createMockController>;

function initialInit() {
  if (typeof globalThis === 'undefined') return undefined;
  const key = '__SWEEPER_MOCK_INIT__';
  const g = globalThis as unknown as {
    [k: string]: { elevationStatus?: { isElevated: boolean }; relaunchResult?: { launched: boolean } | { error: ContractError } } | undefined;
  };
  return g[key];
}

export function createMockController(): SweeperClient & {
  advanceTimeMs: (ms: number) => void;
  invalidateSettings: () => void;
  setElevationStatus: (status: { isElevated: boolean }) => void;
  setRelaunchResult: (result: { launched: boolean } | { error: ContractError }) => void;
  reset: () => void;
} {
  const init = initialInit();
  const state = { nowMs: Date.now(), settingsGeneration: 1, scanSeq: 0, analyzerSeq: 0, logSeq: 0 };
  const ctx = {
    state,
    previewSeq: { value: 0 },
    mutable: {
      settingsState: defaultSettings(),
      elevationStatus: { isElevated: init?.elevationStatus?.isElevated ?? false },
      relaunchResult: init?.relaunchResult ?? ({ launched: true } as const),
    },
    sessions: new Map<string, ScanSessionState>(),
    analyzerSessions: new Map<string, AnalyzerSessionState>(),
    previews: new Map<string, PreviewState>(),
    logs: defaultSeedLogs({ nowMs: state.nowMs }) as LogEntry[],
    bumpSettings: () => {
      state.settingsGeneration += 1;
    },
  };
  return { ...buildScanAnalyzerApi(ctx), ...buildPreviewApi(ctx), ...buildCleanApi(ctx), ...buildConfigApi(ctx), ...buildTestApi(ctx) };
}
