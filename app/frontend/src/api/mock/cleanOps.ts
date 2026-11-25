import type { AnalyzerNodeDto, CleanProgressEvent, RiskDto } from '../client';
import { dedupeAnalyzerNodes, err, isUnknownBlocked, toSkippedReasons } from './helpers';
import { validateSession } from './scanAnalyzerOps';
import { rejectCleanWhileScanRunning } from './scanGate';
import type { AnalyzerSessionState, MockCtx, PreviewState, ScanSessionState } from './types';
import type { ScanItemInfo } from './data';

type Tally = { deletedBytes: bigint; deletedCount: bigint; skippedCount: bigint; completed: bigint; reasons: Record<string, bigint> };
type ProgressInput = { onProgress?: (event: CleanProgressEvent) => void; previewToken: string; targetId: string; completed: bigint; total: number; deletedBytes: bigint; ok: boolean };
type CleanOutcome = { targetId: string; ok: boolean; bytes: bigint; skipReason?: string };
type Revalidation = MockCtx['revalidation'];

function validatePreviewToken(input: { ctx: MockCtx; previewToken: string }): PreviewState {
  const preview = input.ctx.previews.get(input.previewToken);
  if (!preview) err({ code: 'previewTokenInvalid', reason: 'Preview token not found.' });
  if (preview.used) err({ code: 'previewTokenInvalid', reason: 'Preview token already used.' });
  if (input.ctx.state.nowMs - preview.createdAtMs > 10 * 60 * 1000) {
    err({ code: 'previewTokenInvalid', reason: 'Preview token expired.' });
  }
  return preview;
}

function validatePreviewSession(input: { ctx: MockCtx; preview: PreviewState }) {
  const scan = input.ctx.sessions.get(input.preview.sessionId);
  if (scan) {
    validateSession({ nowMs: input.ctx.state.nowMs, settingsGeneration: input.ctx.state.settingsGeneration, createdAtMs: scan.createdAtMs, gen: scan.settingsGeneration });
    return;
  }
  const analyzer = input.ctx.analyzerSessions.get(input.preview.sessionId);
  if (analyzer) {
    validateSession({ nowMs: input.ctx.state.nowMs, settingsGeneration: input.ctx.state.settingsGeneration, createdAtMs: analyzer.createdAtMs, gen: analyzer.settingsGeneration });
    return;
  }
  err({ code: 'sessionInvalid', reason: 'Session not found.' });
}

function assertPreviewGeneration(input: { ctx: MockCtx; preview: PreviewState }) {
  if (input.preview.settingsGeneration === input.ctx.state.settingsGeneration) return;
  err({ code: 'sessionInvalid', reason: 'Session invalidated by settings change.' });
}

function riskAllowed(input: { risk: RiskDto; allowDeleteUnknown: boolean }): boolean {
  if (input.risk === 'Protected') return false;
  return !isUnknownBlocked({ risk: input.risk, allowDeleteUnknown: input.allowDeleteUnknown });
}

function nodeFlagsBlocked(node: AnalyzerNodeDto): boolean {
  if (!node.selectable) return true;
  if (node.displayOnly) return true;
  if (node.isLinkPoint) return true;
  return node.isCloudPlaceholder;
}

function blockedByShape(input: { item?: ScanItemInfo; node?: AnalyzerNodeDto }): boolean {
  if (input.item) return !input.item.ageEligible;
  if (!input.node) return true;
  return nodeFlagsBlocked(input.node);
}

function entryCleanable(input: { item?: ScanItemInfo; node?: AnalyzerNodeDto; allowDeleteUnknown: boolean }): boolean {
  if (blockedByShape(input)) return false;
  const risk = (input.item?.risk ?? input.node?.risk ?? 'Protected') as RiskDto;
  return riskAllowed({ risk, allowDeleteUnknown: input.allowDeleteUnknown });
}

function removeScanItem(input: { session: ScanSessionState; itemId: string }) {
  input.session.itemsById.delete(input.itemId);
  for (const target of input.session.targets) {
    target.items = target.items.filter((entry) => entry.id !== input.itemId);
  }
}

function outcomeBytes(input: { ok: boolean; size?: string | null }): bigint {
  if (!input.ok) return 0n;
  return BigInt(input.size ?? '0');
}

function outcomeTargetId(input: { item?: ScanItemInfo; node?: AnalyzerNodeDto }): string {
  return input.item?.targetId ?? input.node?.targetId ?? 'custom-path';
}

function skippedReason(reason: string): string {
  return `Skipped: ${reason}`;
}

function resolveOutcome(input: { scan?: ScanSessionState; analyzer?: AnalyzerSessionState; itemId: string; allowDeleteUnknown: boolean; revalidation: Revalidation }): CleanOutcome {
  const item = input.scan?.itemsById.get(input.itemId);
  const node = input.analyzer?.nodesById.get(input.itemId);
  const targetId = outcomeTargetId({ item, node });
  const invalidation = input.revalidation.get(input.itemId);
  if (invalidation) {
    return { targetId, ok: false, bytes: 0n, skipReason: skippedReason(invalidation.reason) };
  }
  const ok = entryCleanable({ item, node, allowDeleteUnknown: input.allowDeleteUnknown });
  if (ok && input.scan) removeScanItem({ session: input.scan, itemId: input.itemId });
  return {
    targetId,
    ok,
    bytes: outcomeBytes({ ok, size: item?.size ?? node?.sizeBytes }),
  };
}

function resolveExecuteIds(input: { preview: PreviewState; analyzerSession?: AnalyzerSessionState }): string[] {
  if (!input.analyzerSession) return [...input.preview.itemIds];
  const nodes = input.preview.itemIds.map((id) => input.analyzerSession?.nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
  return dedupeAnalyzerNodes(nodes).map((node) => node.id);
}

function reportProgress(input: ProgressInput) {
  input.onProgress?.({
    previewToken: input.previewToken,
    targetId: input.targetId,
    completedItems: input.completed.toString(),
    totalItems: String(input.total),
    bytesFreedSoFar: input.deletedBytes.toString(),
    status: input.ok ? 'deleted' : 'Skipped',
  });
}

function applyOutcome(input: { tally: Tally; outcome: CleanOutcome }) {
  if (input.outcome.ok) {
    input.tally.deletedBytes += input.outcome.bytes;
    input.tally.deletedCount += 1n;
    return;
  }
  input.tally.skippedCount += 1n;
  const key = input.outcome.skipReason ?? 'Skipped: not eligible';
  input.tally.reasons[key] = (input.tally.reasons[key] ?? 0n) + 1n;
}

function runCleanItems(input: { ctx: MockCtx; previewToken: string; ids: string[]; scan?: ScanSessionState; analyzer?: AnalyzerSessionState; onProgress?: (event: CleanProgressEvent) => void }): Tally {
  const tally: Tally = { deletedBytes: 0n, deletedCount: 0n, skippedCount: 0n, completed: 0n, reasons: {} };
  const allowDeleteUnknown = input.ctx.mutable.settingsState.allowDeleteUnknown;
  for (const itemId of input.ids) {
    const outcome = resolveOutcome({ scan: input.scan, analyzer: input.analyzer, itemId, allowDeleteUnknown, revalidation: input.ctx.revalidation });
    applyOutcome({ tally, outcome });
    tally.completed += 1n;
    reportProgress({ onProgress: input.onProgress, previewToken: input.previewToken, targetId: outcome.targetId, completed: tally.completed, total: input.ids.length, deletedBytes: tally.deletedBytes, ok: outcome.ok });
  }
  return tally;
}

function buildCleanLog(input: { ctx: MockCtx; itemIds: string[]; deletedCount: bigint; deletedBytes: bigint }) {
  const ts = Math.floor(input.ctx.state.nowMs / 1000).toString();
  input.ctx.state.logSeq += 1;
  const id = 'mock-clean-' + String(input.ctx.state.logSeq);
  const name = 'clean-' + id + '.jsonl';
  const intents = input.itemIds.map((itemId) => JSON.stringify({ record: 'intent', target_id: itemId, timestamp: ts }));
  const outcome = JSON.stringify({ record: 'outcome', status: 'completed', deletedCount: input.deletedCount.toString(), bytesFreed: input.deletedBytes.toString(), timestamp: ts });
  const content = [...intents, outcome].join('\n') + '\n';
  return { id, name, modifiedUnixSecs: ts, sizeBytes: String(content.length), content };
}

function abortedClean(reason: string) {
  return {
    bytesFreed: '0',
    deletedCount: '0',
    skippedCount: '1',
    skippedReasons: [{ reason: skippedReason(reason), count: '1' }],
    aborted: true,
    abortReason: `aborted: ${reason}`,
    logId: null,
  };
}

function executeClean(input: { ctx: MockCtx; previewToken: string; onProgress?: (event: CleanProgressEvent) => void }) {
  rejectCleanWhileScanRunning(input.ctx);
  const preview = validatePreviewToken({ ctx: input.ctx, previewToken: input.previewToken });
  validatePreviewSession({ ctx: input.ctx, preview });
  assertPreviewGeneration({ ctx: input.ctx, preview });
  preview.used = true;
  const scan = input.ctx.sessions.get(preview.sessionId);
  const analyzer = input.ctx.analyzerSessions.get(preview.sessionId);
  const ids = resolveExecuteIds({ preview, analyzerSession: analyzer });
  const abortEntry = ids.map((id) => input.ctx.revalidation.get(id)).find((entry) => entry?.abort === true);
  if (abortEntry) return abortedClean(abortEntry.reason);
  const tally = runCleanItems({ ctx: input.ctx, previewToken: input.previewToken, ids, scan, analyzer, onProgress: input.onProgress });
  const log = buildCleanLog({ ctx: input.ctx, itemIds: preview.itemIds, deletedCount: tally.deletedCount, deletedBytes: tally.deletedBytes });
  input.ctx.logs.unshift(log);
  return { bytesFreed: tally.deletedBytes.toString(), deletedCount: tally.deletedCount.toString(), skippedCount: tally.skippedCount.toString(), skippedReasons: toSkippedReasons(tally.reasons), aborted: false, abortReason: null, logId: log.id };
}

export function buildCleanApi(ctx: MockCtx) {
  return {
    async executeClean(previewToken: string, onProgress?: (event: CleanProgressEvent) => void) {
      return executeClean({ ctx, previewToken, onProgress });
    },
  };
}
