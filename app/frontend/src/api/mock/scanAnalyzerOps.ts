import type { AnalyzerProgressEvent, ScanProgressEvent, TierDto } from '../client';
import { err } from './helpers';
import { buildAnalyzerTree, indexAnalyzerNodes, makeScanResults } from './data';
import type { AnalyzerSessionState, MockCtx, ScanSessionState } from './types';

export function isExpired(input: { nowMs: number; createdAtMs: number }): boolean {
  return input.nowMs - input.createdAtMs > 15 * 60 * 1000;
}

export function validateSession(input: { nowMs: number; settingsGeneration: number; createdAtMs: number; gen: number }) {
  if (isExpired({ nowMs: input.nowMs, createdAtMs: input.createdAtMs })) {
    err({ code: 'sessionInvalid', reason: 'Session expired.' });
  }
  if (input.gen !== input.settingsGeneration) {
    err({ code: 'sessionInvalid', reason: 'Session invalidated by settings change.' });
  }
}

export function getValidSessionState<T extends { createdAtMs: number; settingsGeneration: number }>(input: {
  ctx: MockCtx;
  sessions: Map<string, T>;
  sessionId: string;
}): T {
  const session = input.sessions.get(input.sessionId);
  if (!session) err({ code: 'sessionInvalid', reason: 'Session not found.' });
  validateSession({
    nowMs: input.ctx.state.nowMs,
    settingsGeneration: input.ctx.state.settingsGeneration,
    createdAtMs: session.createdAtMs,
    gen: session.settingsGeneration,
  });
  return session;
}

type ScanTarget = { targetId: string; targetName: string; fileCount: string; items: unknown[]; totalBytes: string };
type ScanTotals = { scannedItems: bigint; scannedBytes: bigint };

function targetCounts(target: ScanTarget): { count: bigint; bytes: bigint } {
  const countValue = target.fileCount ? target.fileCount : target.items.length;
  const bytesValue = target.totalBytes ? target.totalBytes : '0';
  return { count: BigInt(countValue), bytes: BigInt(bytesValue) };
}

function scanProgressEvent(input: { sessionId: string; target: ScanTarget; index: number; totalTargets: number; totals: ScanTotals }): ScanProgressEvent {
  return {
    kind: 'progress',
    session_id: input.sessionId,
    completed_targets: input.index + 1,
    total_targets: input.totalTargets,
    current_target_id: input.target.targetId,
    current_target_name: input.target.targetName,
    scanned_items: input.totals.scannedItems.toString(),
    scanned_bytes: input.totals.scannedBytes.toString(),
  };
}

function isCancelled(ctx: MockCtx, sessionId: string): boolean {
  return ctx.sessions.get(sessionId)?.cancelled ?? false;
}

async function emitTargetProgress(input: { ctx: MockCtx; sessionId: string; target: ScanTarget; index: number; totalTargets: number; totals: ScanTotals; onProgress?: (p: ScanProgressEvent) => void }) {
  // Test-only delay keeps a Scan observably Running long enough for Stop E2E.
  const delay = input.ctx.mutable.scanStepDelayMs;
  if (delay > 0) await new Promise<void>((resolve) => setTimeout(resolve, delay));
  if (isCancelled(input.ctx, input.sessionId)) return;
  const counts = targetCounts(input.target);
  input.totals.scannedItems += counts.count;
  input.totals.scannedBytes += counts.bytes;
  input.onProgress?.(scanProgressEvent({ sessionId: input.sessionId, target: input.target, index: input.index, totalTargets: input.totalTargets, totals: input.totals }));
  await Promise.resolve();
}

async function emitScanProgress(input: { ctx: MockCtx; sessionId: string; targets: ScanTarget[]; onProgress?: (p: ScanProgressEvent) => void }): Promise<boolean> {
  input.onProgress?.({ kind: 'started', session_id: input.sessionId });
  const totals = { scannedItems: 0n, scannedBytes: 0n };
  for (let i = 0; i < input.targets.length; i++) {
    if (isCancelled(input.ctx, input.sessionId)) break;
    await emitTargetProgress({ ctx: input.ctx, sessionId: input.sessionId, target: input.targets[i], index: i, totalTargets: input.targets.length, totals, onProgress: input.onProgress });
  }
  if (isCancelled(input.ctx, input.sessionId)) {
    input.onProgress?.({ kind: 'cancelled', session_id: input.sessionId });
    return false;
  }
  if (input.targets.length === 0) {
    input.onProgress?.({ kind: 'progress', session_id: input.sessionId, completed_targets: 0, total_targets: 0, current_target_id: null, current_target_name: null, scanned_items: '0', scanned_bytes: '0' });
    await Promise.resolve();
  }
  if (isCancelled(input.ctx, input.sessionId)) {
    input.onProgress?.({ kind: 'cancelled', session_id: input.sessionId });
    return false;
  }
  input.onProgress?.({ kind: 'completed', session_id: input.sessionId });
  return true;
}

export function buildScanAnalyzerApi(ctx: MockCtx) {
  return {
    async startScan(tiers: TierDto[] | null, onProgress?: (p: ScanProgressEvent) => void) {
      ctx.state.scanSeq += 1;
      const sessionId = 'mock-session-' + String(ctx.state.scanSeq);
      const { targets, itemsById } = makeScanResults({ settingsState: ctx.mutable.settingsState, tiers });
      ctx.sessions.set(sessionId, { id: sessionId, createdAtMs: ctx.state.nowMs, minAgeHours: ctx.mutable.settingsState.minAgeHours, targets, itemsById, settingsGeneration: ctx.state.settingsGeneration, cancelled: false });
      const completed = await emitScanProgress({ ctx, sessionId, targets: targets as ScanTarget[], onProgress });
      if (!completed) err({ code: 'internal', reason: 'scan cancelled' });
      return { sessionId };
    },
    async getScanResults(sessionId: string) {
      const session = getValidSessionState({ ctx, sessions: ctx.sessions, sessionId });
      return { sessionId: session.id, startedUnixSecs: Math.floor(session.createdAtMs / 1000).toString(), minAgeHours: session.minAgeHours, targets: session.targets };
    },
    async cancelScan(sessionId: string) {
      // Idempotent and session-scoped: unknown ids are harmless no-ops, just
      // like the Rust `cancel_scan` command (#94).
      const session = ctx.sessions.get(sessionId);
      if (session) session.cancelled = true;
    },
    cancelActiveScans() {
      for (const session of ctx.sessions.values()) session.cancelled = true;
    },
    async startAnalyzer(root: string | null | undefined, onProgress?: (p: AnalyzerProgressEvent) => void) {
      ctx.state.analyzerSeq += 1;
      const sessionId = 'mock-analyzer-' + String(ctx.state.analyzerSeq);
      const trimmed = root ? root.trim() : '';
      const rootPath = trimmed || 'C:\\';
      onProgress?.({ sessionId, scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: rootPath });
      const tree = buildAnalyzerTree({ rootPath, allowUnknown: ctx.mutable.settingsState.allowDeleteUnknown });
      const nodesById = indexAnalyzerNodes(tree);
      const result = { sessionId, rootPath, totalBytes: tree.sizeBytes, totalItems: tree.itemCount, scannedDirs: '42', scannedFiles: String(nodesById.size), cancelled: false, warnings: [] as string[], root: tree };
      onProgress?.({ sessionId, scannedDirs: '42', scannedFiles: result.scannedFiles, scannedBytes: result.totalBytes, currentPath: null });
      ctx.analyzerSessions.set(sessionId, { id: sessionId, createdAtMs: ctx.state.nowMs, rootPath, result, nodesById, settingsGeneration: ctx.state.settingsGeneration });
      return result;
    },
    async cancelAnalyzer(_sessionId: string) {
      return;
    },
  };
}
