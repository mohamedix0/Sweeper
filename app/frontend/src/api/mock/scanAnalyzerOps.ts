import type { AnalyzerProgressEvent, ScanProgressEvent, TierDto } from '../client';
import { err } from './helpers';
import { parseMockPath } from './pathPolicy';
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

type ScanProgressSink = (p: ScanProgressEvent) => void;
type ScanProgressOutcome = 'completed' | 'cancelled' | 'stale';

function isStale(ctx: MockCtx, session: ScanSessionState): boolean {
  return ctx.sessions.get(session.id) !== session;
}

function emitScanEvent(input: { onProgress: ScanProgressSink | undefined; event: ScanProgressEvent }): void {
  input.onProgress?.(input.event);
}

function emptyTargetsProgress(input: { sessionId: string }): ScanProgressEvent {
  return { kind: 'progress', session_id: input.sessionId, completed_targets: 0, total_targets: 0, current_target_id: null, current_target_name: null, scanned_items: '0', scanned_bytes: '0' };
}

function reportCancellation(input: { session: ScanSessionState; onProgress: ScanProgressSink | undefined }): ScanProgressOutcome {
  emitScanEvent({ onProgress: input.onProgress, event: { kind: 'cancelled', session_id: input.session.id } });
  return 'cancelled';
}

async function emitTargetProgress(input: { ctx: MockCtx; session: ScanSessionState; target: ScanTarget; index: number; totalTargets: number; totals: ScanTotals; onProgress?: ScanProgressSink }): Promise<ScanProgressOutcome> {
  // Test-only delay keeps a Scan observably Running long enough for Stop E2E.
  const delay = input.ctx.mutable.scanStepDelayMs;
  if (delay > 0) await new Promise<void>((resolve) => setTimeout(resolve, delay));
  // A reset/scanSeq reuse can leave a delayed scan resuming under a recycled id.
  if (isStale(input.ctx, input.session)) return 'stale';
  if (input.session.cancelled) return 'cancelled';
  const counts = targetCounts(input.target);
  input.totals.scannedItems += counts.count;
  input.totals.scannedBytes += counts.bytes;
  emitScanEvent({ onProgress: input.onProgress, event: scanProgressEvent({ sessionId: input.session.id, target: input.target, index: input.index, totalTargets: input.totalTargets, totals: input.totals }) });
  await Promise.resolve();
  return isStale(input.ctx, input.session) ? 'stale' : 'completed';
}

async function emitTargetsProgress(input: { ctx: MockCtx; session: ScanSessionState; targets: ScanTarget[]; onProgress?: ScanProgressSink }): Promise<ScanProgressOutcome> {
  const { ctx, session, targets, onProgress } = input;
  const totals = { scannedItems: 0n, scannedBytes: 0n };
  for (let i = 0; i < targets.length; i++) {
    if (isStale(ctx, session)) return 'stale';
    if (session.cancelled) break;
    const outcome = await emitTargetProgress({ ctx, session, target: targets[i], index: i, totalTargets: targets.length, totals, onProgress });
    if (outcome !== 'completed') return outcome;
  }
  return isStale(ctx, session) ? 'stale' : 'completed';
}

async function emitScanProgress(input: { ctx: MockCtx; session: ScanSessionState; targets: ScanTarget[]; onProgress?: ScanProgressSink }): Promise<ScanProgressOutcome> {
  const { ctx, session, targets, onProgress } = input;
  const sessionId = session.id;
  emitScanEvent({ onProgress, event: { kind: 'started', session_id: sessionId } });
  const outcome = await emitTargetsProgress({ ctx, session, targets, onProgress });
  if (isStale(ctx, session) || outcome === 'stale') return 'stale';
  if (outcome === 'cancelled' || session.cancelled) return reportCancellation({ session, onProgress });
  if (targets.length === 0) {
    emitScanEvent({ onProgress, event: emptyTargetsProgress({ sessionId }) });
    await Promise.resolve();
  }
  if (isStale(ctx, session)) return 'stale';
  if (session.cancelled) return reportCancellation({ session, onProgress });
  emitScanEvent({ onProgress, event: { kind: 'completed', session_id: sessionId } });
  return 'completed';
}

export function buildScanAnalyzerApi(ctx: MockCtx) {
  return {
    async startScan(tiers: TierDto[] | null, onProgress?: (p: ScanProgressEvent) => void) {
      ctx.state.scanSeq += 1;
      const sessionId = 'mock-session-' + String(ctx.state.scanSeq);
      const { targets, itemsById } = makeScanResults({ settingsState: ctx.mutable.settingsState, tiers });
      const session: ScanSessionState = { id: sessionId, createdAtMs: ctx.state.nowMs, minAgeHours: ctx.mutable.settingsState.minAgeHours, targets, itemsById, settingsGeneration: ctx.state.settingsGeneration, cancelled: false, running: true };
      ctx.sessions.set(sessionId, session);
      let outcome: ScanProgressOutcome;
      try {
        outcome = await emitScanProgress({ ctx, session, targets: targets as ScanTarget[], onProgress });
      } finally {
        // Only the worker's terminal path retires the active registration (#95),
        // even when cancellation is cooperative or the session went stale.
        session.running = false;
      }
      // A reset may have removed this session or replaced it under the reused
      // id while progress was delayed; a stale scan must not report anything.
      if (isStale(ctx, session)) return { sessionId };
      if (outcome === 'cancelled') err({ code: 'scanCancelled', session_id: sessionId });
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
      // Mirror backend `validate_analyzer_root`: null/empty/whitespace defaults
      // to C:\, while relative / UNC / `..` / device-prefix roots are rejected.
      // Protected container roots stay valid (lexical-only, no Custom Path Policy).
      const trimmed = root ? root.trim() : '';
      const parsed = parseMockPath(trimmed || 'C:\\');
      if (!parsed) err({ code: 'invalidInput', reason: 'invalid analyzer root: path must be an absolute drive path without traversal segments.' });
      const rootPath = parsed.normalized;
      ctx.state.analyzerSeq += 1;
      const sessionId = 'mock-analyzer-' + String(ctx.state.analyzerSeq);
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
