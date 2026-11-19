import type {
  AnalyzerNodeDto,
  PreviewTargetBreakdownDto,
  RiskDto,
  SkippedReasonDto,
  TargetScanResultDto,
} from '../contract/bindings';
import type { Settings, SweeperClient } from './client';
import { createMockController as createMockControllerImpl } from './mock/controller';

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

export type ContractError =
  | { code: 'sessionInvalid'; details: { reason: string } }
  | { code: 'unknownItem'; details: { count: string } }
  | { code: 'previewTokenInvalid'; details: { reason: string } }
  | { code: 'invalidInput'; details: { reason: string } }
  | { code: 'notFound'; details: { reason: string } }
  | { code: 'internal'; details: { reason: string } };

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

export type PreviewCleanResult = {
  previewToken: string;
  breakdown: PreviewTargetBreakdownDto[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
};

export type ExplainResult = { itemId: string; title: string; description: string; risk: RiskDto };

export type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
export type LogContent = { id: string; content: string };

export type MockController = SweeperClient & {
  advanceTimeMs: (ms: number) => void;
  invalidateSettings: () => void;
  setElevationStatus: (status: { isElevated: boolean }) => void;
  setRelaunchResult: (result: { launched: boolean } | { error: ContractError }) => void;
  reset: () => void;
};

export function createMockController(): MockController {
  return createMockControllerImpl() as unknown as MockController;
}

export const mockController = createMockController();
