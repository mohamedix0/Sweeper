import { commands, events } from '../contract/bindings';
import type { ContractError, RiskDto, ScanItemDto, TargetScanResultDto, TierDto } from '../contract/bindings';
import { createMockController, mockController } from './mock';

export type { ContractError, RiskDto, ScanItemDto, TargetScanResultDto, TierDto };

export type ScanProgress =
  | { kind: 'started'; session_id: string }
  | { kind: 'completed'; session_id: string }
  | { kind: 'failed'; session_id: string; error: ContractError };

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

export interface SweeperClient {
  startScan(tiers: TierDto[] | null, onProgress?: (e: ScanProgress) => void): Promise<{ sessionId: string }>;
  getScanResults(sessionId: string): Promise<ScanResults>;
  previewClean(
    sessionId: string,
    itemIds: string[],
    confirmations: { typedConfirmation: string | null } | null,
  ): Promise<PreviewCleanResult>;
  explainItem(itemId: string): Promise<{ itemId: string; title: string; description: string; risk: RiskDto }>;
  getSettings(): Promise<{
    enableTier1: boolean;
    enableTier2: boolean;
    enableTier3: boolean;
    minAgeHours: number;
    allowDeleteUnknown: boolean;
  }>;
  listLogs(): Promise<{ id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null }[]>;
  readLog(logId: string): Promise<{ id: string; content: string }>;
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function shouldUseMockBackend(): boolean {
  const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
  if (env.VITE_SWEEPER_MOCK === '1' || env.VITE_SWEEPER_MOCK === 'true') return true;
  // for test/build environments without tauri, prefer mock unless explicitly forced otherwise
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
    async explainItem(itemId) {
      return unwrap(await commands.explainItem(itemId));
    },
    async getSettings() {
      return unwrap(await commands.getSettings());
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

// exposed for tests
export const _test = { createMockController, mockController };
