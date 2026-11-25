import type { AnalyzerNodeDto, ContractError, Settings } from '../client';
import type { ScanItemInfo } from './data';

export type ScanSessionState = {
  id: string;
  createdAtMs: number;
  minAgeHours: number;
  targets: import('../../contract/bindings').TargetScanResultDto[];
  itemsById: Map<string, ScanItemInfo>;
  settingsGeneration: number;
  /** Set by cancelScan / the host-close hook; stops further progress emission. */
  cancelled: boolean;
  /** True until the scan worker reaches a terminal outcome (completed/cancelled/stale). Mirrors the backend `active_scans` registration. */
  running: boolean;
};

export type AnalyzerSessionState = {
  id: string;
  createdAtMs: number;
  rootPath: string;
  result: import('../client').StartAnalyzerResult;
  nodesById: Map<string, AnalyzerNodeDto>;
  settingsGeneration: number;
};

export type PreviewState = {
  token: string;
  sessionId: string;
  createdAtMs: number;
  used: boolean;
  itemIds: string[];
  settingsGeneration: number;
  previewEstimatedBytes: string;
};

/** Test-controlled re-validation state for one selected item. */
export type RevalidationEntry = {
  reason: string;
  abort: boolean;
};

export type LogEntry = { id: string; name: string; modifiedUnixSecs: string; sizeBytes: string; content: string };

export type MockCtx = {
  state: { nowMs: number; settingsGeneration: number; scanSeq: number; analyzerSeq: number; logSeq: number };
  previewSeq: { value: number };
  mutable: {
    settingsState: Settings;
    elevationStatus: { isElevated: boolean };
    relaunchResult: { launched: boolean } | { error: ContractError };
    /** Test-only delay per scan progress step, used to exercise Stop in E2E. */
    scanStepDelayMs: number;
  };
  sessions: Map<string, ScanSessionState>;
  analyzerSessions: Map<string, AnalyzerSessionState>;
  previews: Map<string, PreviewState>;
  logs: LogEntry[];
  revalidation: Map<string, RevalidationEntry>;
  bumpSettings: () => void;
};
