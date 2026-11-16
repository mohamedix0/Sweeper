import { formatContractError, type SweeperClient } from '../../api/client';
import type { AnalyzerNodeDto, ContractError } from '../../contract/bindings';
import type { PreviewResult } from '../cleanShared';

type PreviewRefs = {
  previewRevision: { current: number };
  activePreviewRequest: { current: number };
};

type PreviewState = {
  client: SweeperClient;
  sessionId: string | null;
  dedupedSelectedNodes: AnalyzerNodeDto[];
  requiresConfirmation: boolean;
  confirmText: string;
  refs: PreviewRefs;
  setError: (v: string | null) => void;
  setInfo: (v: string | null) => void;
  setPreviewLoading: (v: boolean) => void;
  setPreview: (v: PreviewResult | null) => void;
  setCleanSummary: (v: any) => void;
  setCleanProgress: (v: any) => void;
  resetAnalyzerSession: (msg?: string) => void;
};

function confirmationsOf(args: { requiresConfirmation: boolean; confirmText: string }) {
  if (!args.requiresConfirmation) return null;
  return { typedConfirmation: args.confirmText || null };
}

function isStale(args: { refs: PreviewRefs; requestId: number; requestRevision: number; sessionId: string | null; requestSessionId: string }): boolean {
  if (args.refs.activePreviewRequest.current !== args.requestId) return true;
  if (args.refs.previewRevision.current !== args.requestRevision) return true;
  return args.sessionId !== args.requestSessionId;
}

function handlePreviewError(args: { e: unknown; stale: boolean; setError: (v: string | null) => void; setPreview: (v: PreviewResult | null) => void; resetAnalyzerSession: (msg?: string) => void }) {
  if (args.stale) return;
  const ce = args.e as ContractError | undefined;
  args.setError(formatContractError(args.e));
  if (ce?.code === 'sessionInvalid') args.resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
  if (ce?.code === 'previewTokenInvalid') args.setPreview(null);
}

function finishPreviewLoading(args: { refs: PreviewRefs; requestId: number; setPreviewLoading: (v: boolean) => void }) {
  if (args.refs.activePreviewRequest.current !== args.requestId) return;
  args.setPreviewLoading(false);
}

async function requestPreview(args: { state: PreviewState; requestId: number; requestRevision: number; requestSessionId: string }) {
  const confirmations = confirmationsOf({ requiresConfirmation: args.state.requiresConfirmation, confirmText: args.state.confirmText });
  const ids = args.state.dedupedSelectedNodes.map((n) => n.id);
  const res = await args.state.client.previewClean(args.requestSessionId, ids, confirmations);
  if (isStale({ refs: args.state.refs, requestId: args.requestId, requestRevision: args.requestRevision, sessionId: args.state.sessionId, requestSessionId: args.requestSessionId })) return;
  args.state.setPreview({ ...res, issuedAtMs: Date.now() });
}

function onPreviewError(args: { state: PreviewState; e: unknown; requestId: number; requestRevision: number; requestSessionId: string }) {
  handlePreviewError({
    e: args.e,
    stale: isStale({ refs: args.state.refs, requestId: args.requestId, requestRevision: args.requestRevision, sessionId: args.state.sessionId, requestSessionId: args.requestSessionId }),
    setError: args.state.setError,
    setPreview: args.state.setPreview,
    resetAnalyzerSession: args.state.resetAnalyzerSession,
  });
}

export async function runPreviewAction(args: PreviewState): Promise<void> {
  if (!args.sessionId) return;
  const requestId = args.refs.activePreviewRequest.current + 1;
  args.refs.activePreviewRequest.current = requestId;
  const requestRevision = args.refs.previewRevision.current;
  const requestSessionId = args.sessionId;
  args.setError(null);
  args.setInfo(null);
  args.setPreviewLoading(true);
  args.setCleanSummary(null);
  args.setCleanProgress(null);
  try {
    await requestPreview({ state: args, requestId, requestRevision, requestSessionId });
  } catch (e) {
    onPreviewError({ state: args, e, requestId, requestRevision, requestSessionId });
  } finally {
    finishPreviewLoading({ refs: args.refs, requestId, setPreviewLoading: args.setPreviewLoading });
  }
}
