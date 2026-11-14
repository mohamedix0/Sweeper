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
};

export type ScanProgress =
  | { kind: 'started'; session_id: string }
  | { kind: 'completed'; session_id: string }
  | { kind: 'failed'; session_id: string; error: ContractError };

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
  minAgeHours: number;
  allowDeleteUnknown: boolean;
  customPaths: string[];
  staleProjectDays: number;
};

export interface SweeperClient {
  startScan(tiers: TierDto[] | null, onProgress?: (e: ScanProgress) => void): Promise<{ sessionId: string }>;
  getScanResults(sessionId: string): Promise<ScanResults>;
  previewClean(
    sessionId: string,
    itemIds: string[],
    confirmations: { typedConfirmation: string | null } | null,
  ): Promise<PreviewCleanResult>;
  executeClean(previewToken: string, onProgress?: (e: CleanProgressEvent) => void): Promise<ExecuteCleanResponse>;
  startAnalyzer(root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult>;
  explainItem(itemId: string): Promise<{ itemId: string; title: string; description: string; risk: RiskDto }>;
  getSettings(): Promise<Settings>;
  updateSettings(settings: Settings): Promise<Settings>;
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

export function createTauriClient(): SweeperClient {
  return {
    async startScan(tiers, onProgress) {
      let unlisten: (() => void) | null = null;
      try {
        if (onProgress) {
          unlisten = await events.scanProgressEvent.listen((event) => {
            onProgress(event.payload as ScanProgress);
          });
        }
        return unwrap(await commands.startScan(tiers));
      } finally {
        if (unlisten) unlisten();
      }
    },
    async getScanResults(sessionId) {
      return unwrap(await commands.getScanResults(sessionId));
    },
    async previewClean(sessionId, itemIds, confirmations) {
      return unwrap(await commands.previewClean(sessionId, itemIds, confirmations));
    },
    async executeClean(previewToken, onProgress) {
      let unlisten: (() => void) | null = null;
      try {
        if (onProgress) {
          unlisten = await events.cleanProgressEvent.listen((event) => {
            const payload = event.payload as CleanProgressEvent;
            if (payload.previewToken === previewToken) onProgress(payload);
          });
        }
        return unwrap(await commands.executeClean(previewToken));
      } finally {
        if (unlisten) unlisten();
      }
    },
    async startAnalyzer(root, onProgress) {
      let unlisten: (() => void) | null = null;
      try {
        if (onProgress) {
          unlisten = await events.analyzerProgressEvent.listen((event) => {
            onProgress(event.payload as AnalyzerProgressEvent);
          });
        }
        return unwrap(await commands.startAnalyzer(root));
      } finally {
        if (unlisten) unlisten();
      }
    },
    async explainItem(itemId) {
      return unwrap(await commands.explainItem(itemId));
    },
    async getSettings() {
      return unwrap(await commands.getSettings());
    },
    async updateSettings(settings) {
      return unwrap(await commands.updateSettings(settings));
    },
    async validateCustomPath(path) {
      return unwrap(await commands.validateCustomPath(path));
    },
    async listLogs() {
      return unwrap(await commands.listLogs());
    },
    async readLog(logId) {
      return unwrap(await commands.readLog(logId));
    },
  };
}

export function createClient(): SweeperClient {
  if (shouldUseMockBackend()) return mockController;
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