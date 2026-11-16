import { formatContractError, type CleanProgressEvent, type ExecuteCleanResponse, type SweeperClient } from '../../api/client';
import type { ContractError } from '../../contract/bindings';
import type { PreviewResult } from '../cleanShared';

type CleanArgs = {
  client: SweeperClient;
  preview: PreviewResult | null;
  setError: (v: string | null) => void;
  setInfo: (v: string | null) => void;
  setCleaning: (v: boolean) => void;
  setCleanProgress: (v: CleanProgressEvent | null) => void;
  setCleanSummary: (v: ExecuteCleanResponse | null) => void;
  setPreview: (v: PreviewResult | null) => void;
  setSelectedIds: (v: Set<string>) => void;
  setConfirmText: (v: string) => void;
  resetAnalyzerSession: (msg?: string) => void;
};

function handleCleanError(args: { e: unknown; setError: (v: string | null) => void; setInfo: (v: string | null) => void; setPreview: (v: PreviewResult | null) => void; resetAnalyzerSession: (msg?: string) => void }) {
  const ce = args.e as ContractError | undefined;
  args.setError(formatContractError(args.e));
  if (ce?.code === 'sessionInvalid') args.resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
  if (ce?.code === 'previewTokenInvalid') {
    args.setPreview(null);
    args.setInfo('Preview token is invalid, expired, or already used. Run Review & Clean again.');
  }
}

export async function runCleanAction(args: CleanArgs): Promise<void> {
  if (!args.preview) return;
  args.setError(null);
  args.setInfo(null);
  args.setCleaning(true);
  args.setCleanProgress(null);
  args.setCleanSummary(null);
  try {
    const res = await args.client.executeClean(args.preview.previewToken, (ev) => args.setCleanProgress(ev));
    args.setCleanSummary(res);
    args.setPreview(null);
    args.setSelectedIds(new Set());
    args.setConfirmText('');
    args.setInfo('Clean completed. Review the summary and Activity History below.');
  } catch (e) {
    handleCleanError({ e, setError: args.setError, setInfo: args.setInfo, setPreview: args.setPreview, resetAnalyzerSession: args.resetAnalyzerSession });
  } finally {
    args.setCleaning(false);
  }
}
