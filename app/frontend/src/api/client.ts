import { commands, events } from '../contract/bindings';
import type {
  AnalyzerNodeDto,
  ContractError,
  RiskDto,
  ScanItemDto,
  SkippedReasonDto,
  TargetScanResultDto,
  TierDto,
} from '../contract/bindings';
import { mockController } from './mock';

export type { AnalyzerNodeDto, ContractError, RiskDto, ScanItemDto, SkippedReasonDto, TargetScanResultDto, TierDto };

export type CleanProgressEvent = {
  previewToken: string;
  targetId: string;
  completedItems: string;
  totalItems: string;
  bytesFreedSoFar: string;
  status: string;
};

export type ExecuteCleanResponse = {
  bytesFreed: string;
  deletedCount: string;
  skippedCount: string;
  skippedReasons: SkippedReasonDto[];
  aborted: boolean;
  abortReason: string | null;
  logId: string | null;
  journalErrors?: string[];
};

export type ElevationStatus = { isElevated: boolean };
export type RelaunchResult = { launched: boolean };

export type ScanProgressEvent =
  | { kind: 'started'; session_id: string }
  | {
      kind: 'progress';
      session_id: string;
      completed_targets: number;
      total_targets: number;
      current_target_id: string | null;
      current_target_name: string | null;
      scanned_items: string;
      scanned_bytes: string;
    }
  | { kind: 'cancelled'; session_id: string }
  | { kind: 'completed'; session_id: string }
  | { kind: 'failed'; session_id: string; error: ContractError };

export type ScanProgress = ScanProgressEvent;

export type AnalyzerProgressEvent = {
  sessionId: string;
  scannedDirs: string;
  scannedFiles: string;
  scannedBytes: string;
  currentPath: string | null;
};

export type ScanResults = {
  sessionId: string;
  startedUnixSecs: string;
  minAgeHours: number;
  targets: TargetScanResultDto[];
};

export type PreviewCleanResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  skippedCount?: string;
  skippedReasons?: string[];
};

export type StartAnalyzerResult = {
  sessionId: string;
  rootPath: string;
  totalBytes: string;
  totalItems: string;
  scannedDirs: string;
  scannedFiles: string;
  cancelled: boolean;
  warnings: string[];
  root: AnalyzerNodeDto;
};

export type Settings = {
  enableTier1: boolean;
  enableTier2: boolean;
  enableTier3: boolean;
  enableElevationTargets: boolean;
  minAgeHours: number;
  allowDeleteUnknown: boolean;
  customPaths: string[];
  staleProjectDays: number;
};

export interface SweeperClient {
  startScan(tiers: TierDto[] | null, onProgress?: (e: ScanProgressEvent) => void): Promise<{ sessionId: string }>;
  getScanResults(sessionId: string): Promise<ScanResults>;
  cancelScan(sessionId: string): Promise<void>;
  previewClean(
    sessionId: string,
    itemIds: string[],
    confirmations: { typedConfirmation: string | null } | null,
  ): Promise<PreviewCleanResult>;
  executeClean(previewToken: string, onProgress?: (e: CleanProgressEvent) => void): Promise<ExecuteCleanResponse>;
  startAnalyzer(root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult>;
  cancelAnalyzer(sessionId: string): Promise<void>;
  explainItem(itemId: string): Promise<{ itemId: string; title: string; description: string; risk: RiskDto }>;
  getSettings(): Promise<Settings>;
  updateSettings(settings: Settings): Promise<Settings>;
  getElevationStatus(): Promise<ElevationStatus>;
  relaunchElevated(): Promise<RelaunchResult>;
  validateCustomPath(path: string): Promise<{ path: string; allowed: boolean; reason: string }>;
  listLogs(): Promise<{ id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null }[]>;
  readLog(logId: string): Promise<{ id: string; content: string }>;
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function shouldUseMockBackend(): boolean {
  const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
  if (env.VITE_SWEEPER_MOCK === '1' || env.VITE_SWEEPER_MOCK === 'true') return true;
  if (env.MODE === 'test') return true;
  return !isTauriRuntime();
}

function unwrap<T>(res: { status: 'ok'; data: T } | { status: 'error'; error: ContractError }): T {
  if (res.status === 'ok') return res.data;
  throw res.error;
}

function normalizeSettings(settings: Settings & { enableElevationTargets?: boolean }): Settings {
  return { ...settings, enableElevationTargets: settings.enableElevationTargets ?? false };
}

async function withScanProgress<T>(onProgress: ((e: ScanProgressEvent) => void) | undefined, run: () => Promise<{ status: 'ok'; data: T } | { status: 'error'; error: ContractError }>): Promise<T> {
  let unlisten: (() => void) | null = null;
  try {
    if (onProgress) unlisten = await events.scanProgressEvent.listen((event) => onProgress(event.payload as ScanProgressEvent));
    return unwrap(await run());
  } finally { if (unlisten) unlisten(); }
}

async function withCleanProgress<T>(previewToken: string, onProgress: ((e: CleanProgressEvent) => void) | undefined, run: () => Promise<{ status: 'ok'; data: T } | { status: 'error'; error: ContractError }>): Promise<T> {
  let unlisten: (() => void) | null = null;
  try {
    if (onProgress) unlisten = await events.cleanProgressEvent.listen((event) => { const payload = event.payload as CleanProgressEvent; if (payload.previewToken === previewToken) onProgress(payload); });
    return unwrap(await run());
  } finally { if (unlisten) unlisten(); }
}

async function withAnalyzerProgress<T>(onProgress: ((e: AnalyzerProgressEvent) => void) | undefined, run: () => Promise<{ status: 'ok'; data: T } | { status: 'error'; error: ContractError }>): Promise<T> {
  let unlisten: (() => void) | null = null;
  try {
    if (onProgress) unlisten = await events.analyzerProgressEvent.listen((event) => onProgress(event.payload as AnalyzerProgressEvent));
    return unwrap(await run());
  } finally { if (unlisten) unlisten(); }
}

export function createTauriClient(): SweeperClient {
  return {
    startScan: (tiers, onProgress) => withScanProgress(onProgress, () => commands.startScan(tiers)),
    getScanResults: async (sessionId) => unwrap(await commands.getScanResults(sessionId)),
    cancelScan: async (sessionId) => { unwrap(await commands.cancelScan(sessionId)); },
    previewClean: async (sessionId, itemIds, confirmations) => unwrap(await commands.previewClean(sessionId, itemIds, confirmations)),
    executeClean: (previewToken, onProgress) => withCleanProgress(previewToken, onProgress, () => commands.executeClean(previewToken)),
    startAnalyzer: (root, onProgress) => withAnalyzerProgress(onProgress, () => commands.startAnalyzer(root)),
    cancelAnalyzer: async (sessionId) => { unwrap(await commands.cancelAnalyzer(sessionId)); },
    explainItem: async (itemId) => unwrap(await commands.explainItem(itemId)),
    getSettings: async () => normalizeSettings(unwrap(await commands.getSettings()) as Settings & { enableElevationTargets?: boolean }),
    updateSettings: async (settings) => normalizeSettings(unwrap(await commands.updateSettings(settings)) as Settings & { enableElevationTargets?: boolean }),
    getElevationStatus: async () => unwrap(await commands.getElevationStatus()),
    relaunchElevated: async () => unwrap(await commands.relaunchElevated()),
    validateCustomPath: async (path) => unwrap(await commands.validateCustomPath(path)),
    listLogs: async () => unwrap(await commands.listLogs()),
    readLog: async (logId) => unwrap(await commands.readLog(logId)),
  };
}

export function createClient(): SweeperClient {
  if (shouldUseMockBackend()) {
    if (typeof window !== 'undefined') {
      (window as unknown as { __sweeperMock?: SweeperClient }).__sweeperMock = mockController;
    }
    return mockController;
  }
  return createTauriClient();
}

export function isContractError(e: unknown): e is ContractError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'code' in e &&
    typeof (e as { code: unknown }).code === 'string' &&
    'details' in e
  );
}

export function formatContractError(e: unknown): string {
  if (isContractError(e)) {
    if (e.code === 'unknownItem') return `Unknown item(s): ${e.details.count}`;
    return e.details.reason || e.code;
  }
  if (e instanceof Error) return e.message;
  return 'An unexpected error occurred.';
}