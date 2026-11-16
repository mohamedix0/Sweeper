import type { AnalyzerNodeDto, PreviewCleanResult } from '../client';
import { analyzerBreakdown, dedupeAnalyzerNodes, err, scanBreakdown, sumNodeBytes, toSkippedReasons } from './helpers';
import { getValidSessionState, validateSession } from './scanAnalyzerOps';
import type { AnalyzerSessionState, MockCtx, PreviewState, ScanSessionState } from './types';
import type { ScanItemInfo } from './data';

function requireTyped(confirmations: { typedConfirmation: string | null } | null | undefined) {
  if ((confirmations?.typedConfirmation ?? null) !== 'DELETE') {
    err({ code: 'invalidInput', reason: 'Typed confirmation required: enter DELETE to continue.' });
  }
}

function validateScanItem(item: ScanItemInfo, allowDeleteUnknown: boolean) {
  if (item.risk === 'Protected') err({ code: 'invalidInput', reason: 'Protected items cannot be selected.' });
  if (!item.ageEligible) err({ code: 'invalidInput', reason: 'Ineligible items cannot be selected.' });
  if (item.risk === 'Unknown' && !allowDeleteUnknown) {
    err({ code: 'invalidInput', reason: 'Unknown items are gated by allow_delete_unknown.' });
  }
}

function selectScanItems(session: ScanSessionState, itemIds: string[], allowDeleteUnknown: boolean, confirmations?: { typedConfirmation: string | null } | null) {
  if (!itemIds.length) err({ code: 'invalidInput', reason: 'No items selected.' });
  const selected: ScanItemInfo[] = [];
  let unknownCount = 0;
  let needsConfirm = false;
  for (const id of itemIds) {
    const item = session.itemsById.get(id);
    if (!item) {
      unknownCount += 1;
      continue;
    }
    validateScanItem(item, allowDeleteUnknown);
    selected.push(item);
    if (item.risk === 'Unknown') needsConfirm = true;
    if (item.targetId.startsWith('custom-path')) needsConfirm = true;
  }
  if (unknownCount > 0) err({ code: 'unknownItem', count: String(unknownCount) });
  if (needsConfirm) requireTyped(confirmations);
  return selected;
}

function analyzerEligible(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): boolean {
  if (!node.selectable) return false;
  if (node.displayOnly) return false;
  if (node.risk === 'Protected') return false;
  if (node.isLinkPoint) return false;
  if (node.isCloudPlaceholder) return false;
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return true;
}

function selectAnalyzerNodes(session: AnalyzerSessionState, itemIds: string[], allowDeleteUnknown: boolean, confirmations?: { typedConfirmation: string | null } | null) {
  if (!itemIds.length) err({ code: 'invalidInput', reason: 'No items selected.' });
  const selected: AnalyzerNodeDto[] = [];
  let unknownCount = 0;
  let needsConfirm = false;
  for (const id of itemIds) {
    const node = session.nodesById.get(id);
    if (!node) {
      unknownCount += 1;
      continue;
    }
    if (node.risk === 'Unknown' && !allowDeleteUnknown) {
      err({ code: 'invalidInput', reason: 'Unknown items are gated by allow_delete_unknown.' });
    }
    if (!analyzerEligible(node, true)) {
      err({ code: 'invalidInput', reason: 'Selected analyzer node is not eligible for Review & Clean.' });
    }
    selected.push(node);
    if (node.risk === 'Unknown') needsConfirm = true;
    if (!node.targetId) needsConfirm = true;
  }
  if (unknownCount > 0) err({ code: 'unknownItem', count: String(unknownCount) });
  const deduped = dedupeAnalyzerNodes(selected);
  if (needsConfirm) requireTyped(confirmations);
  return deduped;
}

function storePreview(ctx: MockCtx, sessionId: string, itemIds: string[], bytes: string): string {
  ctx.previewSeq.value += 1;
  const token = 'mock-preview-' + String(ctx.previewSeq.value);
  ctx.previews.set(token, {
    token,
    sessionId,
    createdAtMs: ctx.state.nowMs,
    used: false,
    itemIds,
    settingsGeneration: ctx.state.settingsGeneration,
    previewEstimatedBytes: bytes,
  });
  return token;
}

type PreviewBuild = {
  sessionId: string;
  storedIds: string[];
  count: number;
  bytes: string;
  breakdown: PreviewCleanResult['breakdown'];
};

function issuePreview(ctx: MockCtx, build: PreviewBuild): PreviewCleanResult {
  const token = storePreview(ctx, build.sessionId, build.storedIds, build.bytes);
  return {
    previewToken: token,
    breakdown: build.breakdown,
    totalSelectedCount: BigInt(build.count).toString(),
    totalSelectedBytes: build.bytes,
  };
}

function buildPreview(
  ctx: MockCtx,
  input: { kind: 'scan' | 'analyzer'; sessionId: string; itemIds: string[]; confirmations?: { typedConfirmation: string | null } | null },
): PreviewBuild {
  if (input.kind === 'scan') {
    const session = getValidSessionState({ ctx, sessions: ctx.sessions, sessionId: input.sessionId });
    const selected = selectScanItems(session, input.itemIds, ctx.mutable.settingsState.allowDeleteUnknown, input.confirmations);
    const { breakdown, totalBytes } = scanBreakdown(selected.map((item) => ({ item })));
    return { sessionId: session.id, storedIds: [...input.itemIds], count: selected.length, bytes: totalBytes.toString(), breakdown };
  }
  const session = getValidSessionState({ ctx, sessions: ctx.analyzerSessions, sessionId: input.sessionId });
  const deduped = selectAnalyzerNodes(session, input.itemIds, ctx.mutable.settingsState.allowDeleteUnknown, input.confirmations);
  const bytes = sumNodeBytes(deduped);
  return { sessionId: session.id, storedIds: deduped.map((node) => node.id), count: deduped.length, bytes, breakdown: analyzerBreakdown(deduped) };
}

function getAnySessionMeta(ctx: MockCtx, sessionId: string) {
  const scan = ctx.sessions.get(sessionId);
  if (scan) {
    validateSession({ nowMs: ctx.state.nowMs, settingsGeneration: ctx.state.settingsGeneration, createdAtMs: scan.createdAtMs, gen: scan.settingsGeneration });
    return scan;
  }
  const analyzer = ctx.analyzerSessions.get(sessionId);
  if (analyzer) {
    validateSession({ nowMs: ctx.state.nowMs, settingsGeneration: ctx.state.settingsGeneration, createdAtMs: analyzer.createdAtMs, gen: analyzer.settingsGeneration });
    return analyzer;
  }
  err({ code: 'sessionInvalid', reason: 'Session not found.' });
}

function validatePreviewToken(ctx: MockCtx, token: string): PreviewState {
  const preview = ctx.previews.get(token);
  if (!preview) err({ code: 'previewTokenInvalid', reason: 'Preview token not found.' });
  if (preview.used) err({ code: 'previewTokenInvalid', reason: 'Preview token already used.' });
  if (ctx.state.nowMs - preview.createdAtMs > 10 * 60 * 1000) {
    err({ code: 'previewTokenInvalid', reason: 'Preview token expired.' });
  }
  return preview;
}

function isUnknownBlocked(risk: ScanItemInfo['risk'] | undefined, allowUnknown: boolean): boolean {
  if (risk !== 'Unknown') return false;
  return !allowUnknown;
}

function isScanItemCleanable(item: ScanItemInfo | undefined, allowUnknown: boolean): boolean {
  if (!item) return false;
  if (!item.ageEligible) return false;
  if (item.risk === 'Protected') return false;
  if (isUnknownBlocked(item.risk, allowUnknown)) return false;
  return true;
}

function removeScanItem(scanSession: ScanSessionState, id: string) {
  scanSession.itemsById.delete(id);
  for (const target of scanSession.targets) target.items = target.items.filter((entry) => entry.id !== id);
}

function cleanEntryOutcome(ctx: MockCtx, input: { scan?: ScanSessionState; analyzer?: AnalyzerSessionState; id: string }) {
  const allowUnknown = ctx.mutable.settingsState.allowDeleteUnknown;
  if (input.scan) {
    const item = input.scan.itemsById.get(input.id);
    const ok = isScanItemCleanable(item, allowUnknown);
    if (ok) removeScanItem(input.scan, input.id);
    return { targetId: item?.targetId ?? 'unknown', ok, bytes: ok ? BigInt(item?.size ?? '0') : 0n };
  }
  const node = input.analyzer?.nodesById.get(input.id);
  const ok = node ? analyzerEligible(node, allowUnknown) : false;
  return { targetId: node?.targetId ?? 'custom-path', ok, bytes: ok ? BigInt(node?.sizeBytes ?? '0') : 0n };
}

function buildCleanLog(ctx: MockCtx, itemIds: string[], deletedCount: bigint, deletedBytes: bigint) {
  const ts = Math.floor(ctx.state.nowMs / 1000).toString();
  ctx.state.logSeq += 1;
  const id = 'mock-clean-' + String(ctx.state.logSeq);
  const name = 'clean-' + id + '.jsonl';
  const intents = itemIds.map((itemId) => JSON.stringify({ record: 'intent', target_id: itemId, timestamp: ts }));
  const outcome = JSON.stringify({ record: 'outcome', status: 'completed', deletedCount: deletedCount.toString(), bytesFreed: deletedBytes.toString(), timestamp: ts });
  const content = [...intents, outcome].join('\n') + '\n';
  return { id, name, modifiedUnixSecs: ts, sizeBytes: String(content.length), content };
}

function executeClean(ctx: MockCtx, previewToken: string, onProgress?: (e: import('../client').CleanProgressEvent) => void) {
  const preview = validatePreviewToken(ctx, previewToken);
  getAnySessionMeta(ctx, preview.sessionId);
  if (preview.settingsGeneration !== ctx.state.settingsGeneration) {
    err({ code: 'sessionInvalid', reason: 'Session invalidated by settings change.' });
  }
  preview.used = true;
  const scanSession = ctx.sessions.get(preview.sessionId);
  const analyzerSession = ctx.analyzerSessions.get(preview.sessionId);
  let ids = [...preview.itemIds];
  if (analyzerSession) ids = dedupeAnalyzerNodes(preview.itemIds.map((id) => analyzerSession.nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[]).map((node) => node.id);
  const reasons: Record<string, bigint> = {};
  let deletedBytes = 0n;
  let deletedCount = 0n;
  let skippedCount = 0n;
  let completed = 0n;
  for (const id of ids) {
    const result = cleanEntryOutcome(ctx, { scan: scanSession, analyzer: analyzerSession, id });
    if (result.ok) {
      deletedBytes += result.bytes;
      deletedCount += 1n;
    } else {
      skippedCount += 1n;
      const key = 'Skipped: not eligible';
      reasons[key] = (reasons[key] ?? 0n) + 1n;
    }
    completed += 1n;
    onProgress?.({ previewToken, targetId: result.targetId, completedItems: completed.toString(), totalItems: String(ids.length), bytesFreedSoFar: deletedBytes.toString(), status: result.ok ? 'deleted' : 'Skipped' });
  }
  const log = buildCleanLog(ctx, preview.itemIds, deletedCount, deletedBytes);
  ctx.logs.unshift(log);
  return { bytesFreed: deletedBytes.toString(), deletedCount: deletedCount.toString(), skippedCount: skippedCount.toString(), skippedReasons: toSkippedReasons(reasons), aborted: false, abortReason: null, logId: log.id };
}

export function buildCleanApi(ctx: MockCtx) {
  return {
    async previewClean(sessionId: string, itemIds: string[], confirmations?: { typedConfirmation: string | null } | null) {
      const opts = confirmations ?? null;
      if (ctx.sessions.has(sessionId)) return issuePreview(ctx, buildPreview(ctx, { kind: 'scan', sessionId, itemIds, confirmations: opts }));
      if (ctx.analyzerSessions.has(sessionId)) return issuePreview(ctx, buildPreview(ctx, { kind: 'analyzer', sessionId, itemIds, confirmations: opts }));
      err({ code: 'sessionInvalid', reason: 'Session not found.' });
    },
    async executeClean(previewToken: string, onProgress?: (e: import('../client').CleanProgressEvent) => void) {
      return executeClean(ctx, previewToken, onProgress);
    },
  };
}
