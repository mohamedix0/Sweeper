import type { AnalyzerNodeDto, ContractError, PreviewCleanResult, ScanProgressEvent, Settings, SweeperClient, TierDto } from '../client';
import { analyzerBreakdown, dedupeAnalyzerNodes, err, scanBreakdown, sumNodeBytes, toSkippedReasons } from './helpers';
import { buildAnalyzerTree, defaultSeedLogs, indexAnalyzerNodes, makeScanResults, type ScanItemInfo } from './data';

type SessionState = { id: string; createdAtMs: number; minAgeHours: number; targets: import('../../contract/bindings').TargetScanResultDto[]; itemsById: Map<string, ScanItemInfo>; settingsGeneration: number };
type AnalyzerSessionState = { id: string; createdAtMs: number; rootPath: string; result: import('../client').StartAnalyzerResult; nodesById: Map<string, AnalyzerNodeDto>; settingsGeneration: number };
type PreviewState = { token: string; sessionId: string; createdAtMs: number; used: boolean; itemIds: string[]; settingsGeneration: number; previewEstimatedBytes: string };
type LogEntry = { id: string; name: string; modifiedUnixSecs: string; sizeBytes: string; content: string };

export type MockController = ReturnType<typeof createMockController>;

function defaultSettings(): Settings { return { enableTier1: true, enableTier2: true, enableTier3: false, enableElevationTargets: false, minAgeHours: 24, allowDeleteUnknown: false, customPaths: [], staleProjectDays: 30 }; }
function initialInit() { return typeof globalThis !== 'undefined' ? (globalThis as { __SWEEPER_MOCK_INIT__?: { elevationStatus?: { isElevated: boolean }; relaunchResult?: { launched: boolean } | { error: ContractError } } }).__SWEEPER_MOCK_INIT__ : undefined; }
function isExpired(nowMs: number, createdAtMs: number) { return nowMs - createdAtMs > 15 * 60 * 1000; }
function validateSession(nowMs: number, settingsGeneration: number, createdAtMs: number, gen: number) { if (isExpired(nowMs, createdAtMs)) err('sessionInvalid', 'Session expired.'); if (gen !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.'); }

function scanProgress(args: { sessionId: string; target: { targetId: string; targetName: string; fileCount: string; items: unknown[]; totalBytes: string }; index: number; totalTargets: number; scannedItems: bigint; scannedBytes: bigint }): ScanProgressEvent {
  return { kind: 'progress', session_id: args.sessionId, completed_targets: args.index + 1, total_targets: args.totalTargets, current_target_id: args.target.targetId, current_target_name: args.target.targetName, scanned_items: args.scannedItems.toString(), scanned_bytes: args.scannedBytes.toString() };
}

async function emitScanProgress(args: { sessionId: string; targets: Array<{ targetId: string; targetName: string; fileCount: string; items: unknown[]; totalBytes: string }>; onProgress?: (p: ScanProgressEvent) => void }) {
  args.onProgress?.({ kind: 'started', session_id: args.sessionId });
  let scannedItems = 0n; let scannedBytes = 0n;
  for (let i = 0; i < args.targets.length; i++) {
    const target = args.targets[i];
    scannedItems += BigInt(target.fileCount || target.items.length || 0);
    scannedBytes += BigInt(target.totalBytes || '0');
    args.onProgress?.(scanProgress({ sessionId: args.sessionId, target, index: i, totalTargets: args.targets.length, scannedItems, scannedBytes }));
    await Promise.resolve();
  }
  if (args.targets.length === 0) {
    args.onProgress?.({ kind: 'progress', session_id: args.sessionId, completed_targets: 0, total_targets: 0, current_target_id: null, current_target_name: null, scanned_items: '0', scanned_bytes: '0' });
    await Promise.resolve();
  }
  args.onProgress?.({ kind: 'completed', session_id: args.sessionId });
}

function validateScanItem(item: ScanItemInfo, allowDeleteUnknown: boolean) {
  if (item.risk === 'Protected') err('invalidInput', 'Protected items cannot be selected.');
  if (!item.ageEligible) err('invalidInput', 'Ineligible items cannot be selected.');
  if (item.risk === 'Unknown' && !allowDeleteUnknown) err('invalidInput', 'Unknown items are gated by allow_delete_unknown.');
}

function needsTyped(confirmations: { typedConfirmation: string | null } | null) { return (confirmations?.typedConfirmation ?? null) !== 'DELETE'; }
function requireTyped(confirmations: { typedConfirmation: string | null } | null) { if (needsTyped(confirmations)) err('invalidInput', 'Typed confirmation required: enter DELETE to continue.'); }

function previewScanSelection(args: { session: SessionState; itemIds: string[]; allowDeleteUnknown: boolean; confirmations: { typedConfirmation: string | null } | null }) {
  if (!args.itemIds.length) err('invalidInput', 'No items selected.');
  let unknownCount = 0; const selected: Array<{ item: ScanItemInfo }> = [];
  for (const id of args.itemIds) {
    const item = args.session.itemsById.get(id);
    if (!item) { unknownCount += 1; continue; }
    validateScanItem(item, args.allowDeleteUnknown);
    selected.push({ item });
  }
  if (unknownCount > 0) err('unknownItem', String(unknownCount));
  const needsConfirm = selected.some((x) => x.item.risk === 'Unknown' || x.item.targetId.startsWith('custom-path'));
  if (needsConfirm) requireTyped(args.confirmations);
  return selected;
}

function analyzerNodeEligible(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): boolean {
  if (!node.selectable || node.displayOnly || node.risk === 'Protected' || node.isLinkPoint || node.isCloudPlaceholder) return false;
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return true;
}

function validateAnalyzerNode(node: AnalyzerNodeDto, allowDeleteUnknown: boolean) {
  if (node.risk === 'Unknown' && !allowDeleteUnknown) err('invalidInput', 'Unknown items are gated by allow_delete_unknown.');
  if (!analyzerNodeEligible(node, true)) err('invalidInput', 'Selected analyzer node is not eligible for Review & Clean.');
  if (!analyzerNodeEligible(node, allowDeleteUnknown)) err('invalidInput', 'Unknown items are gated by allow_delete_unknown.');
}

function selectionNeedsConfirmation(nodes: AnalyzerNodeDto[], scanItems?: ScanItemInfo[]): boolean {
  if (scanItems && scanItems.some((i) => i.risk === 'Unknown' || i.targetId.startsWith('custom-path'))) return true;
  return nodes.some((n) => n.risk === 'Unknown' || !n.targetId);
}

function previewAnalyzerSelection(args: { session: AnalyzerSessionState; itemIds: string[]; allowDeleteUnknown: boolean; confirmations: { typedConfirmation: string | null } | null }) {
  if (!args.itemIds.length) err('invalidInput', 'No items selected.');
  let unknownCount = 0; const selected: AnalyzerNodeDto[] = [];
  for (const id of args.itemIds) {
    const node = args.session.nodesById.get(id);
    if (!node) { unknownCount += 1; continue; }
    validateAnalyzerNode(node, args.allowDeleteUnknown);
    selected.push(node);
  }
  if (unknownCount > 0) err('unknownItem', String(unknownCount));
  const deduped = dedupeAnalyzerNodes(selected);
  if (selectionNeedsConfirmation(deduped)) requireTyped(args.confirmations);
  return deduped;
}

function canCleanScanItem(item: ScanItemInfo | undefined, allowDeleteUnknown: boolean): boolean {
  if (!item) return false;
  if (!item.ageEligible || item.risk === 'Protected') return false;
  if (item.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return true;
}

function removeScanItem(scanSession: SessionState, id: string) {
  scanSession.itemsById.delete(id);
  for (const t of scanSession.targets) t.items = t.items.filter((i) => i.id !== id);
}

function cleanOne(args: { id: string; scanSession?: SessionState; analyzerSession?: AnalyzerSessionState; allowDeleteUnknown: boolean }) {
  if (args.scanSession) {
    const item = args.scanSession.itemsById.get(args.id);
    const ok = canCleanScanItem(item, args.allowDeleteUnknown);
    if (ok && item) removeScanItem(args.scanSession, args.id);
    return { targetId: item?.targetId ?? 'unknown', ok, bytes: ok && item ? BigInt(item.size) : 0n };
  }
  const node = args.analyzerSession?.nodesById.get(args.id);
  const ok = analyzerNodeEligible(node as AnalyzerNodeDto, args.allowDeleteUnknown);
  return { targetId: node?.targetId ?? 'custom-path', ok, bytes: ok && node ? BigInt(node.sizeBytes) : 0n };
}

function buildCleanLog(args: { itemIds: string[]; deletedCount: bigint; deletedBytes: bigint; nowMs: number; logSeq: number }) {
  const ts = Math.floor(args.nowMs / 1000).toString();
  const id = `mock-clean-${args.logSeq}`;
  const name = `clean-${id}.jsonl`;
  const content = [...args.itemIds.map((itemId) => JSON.stringify({ record: 'intent', target_id: itemId, timestamp: ts })), JSON.stringify({ record: 'outcome', status: 'completed', deletedCount: args.deletedCount.toString(), bytesFreed: args.deletedBytes.toString(), timestamp: ts })].join('\n') + '\n';
  return { log: { id, name, modifiedUnixSecs: ts, sizeBytes: String(content.length), content } as LogEntry };
}

function validatePreviewToken(previews: Map<string, PreviewState>, token: string, nowMs: number) {
  const p = previews.get(token); if (!p) err('previewTokenInvalid', 'Preview token not found.');
  if (p.used) err('previewTokenInvalid', 'Preview token already used.');
  if (nowMs - p.createdAtMs > 10 * 60 * 1000) err('previewTokenInvalid', 'Preview token expired.');
  return p;
}

export function createMockController(): SweeperClient & { advanceTimeMs: (ms: number) => void; invalidateSettings: () => void; setElevationStatus: (status: { isElevated: boolean }) => void; setRelaunchResult: (result: { launched: boolean } | { error: ContractError }) => void; reset: () => void } {
  const init = initialInit();
  let nowMs = Date.now();
  let settingsGeneration = 1;
  let settingsState = defaultSettings();
  let elevationStatus = { isElevated: init?.elevationStatus?.isElevated ?? false };
  let relaunchResult: { launched: boolean } | { error: ContractError } = init?.relaunchResult ?? { launched: true };
  const sessions = new Map<string, SessionState>();
  const analyzerSessions = new Map<string, AnalyzerSessionState>();
  const previews = new Map<string, PreviewState>();
  let scanSeq = 0; let analyzerSeq = 0; let previewSeq = 0; let logSeq = 0;
  const logs: LogEntry[] = defaultSeedLogs(nowMs);

  const bumpSettings = () => { settingsGeneration += 1; };
  const getScanSession = (sessionId: string) => { const s = sessions.get(sessionId); if (!s) err('sessionInvalid', 'Session not found.'); validateSession(nowMs, settingsGeneration, s.createdAtMs, s.settingsGeneration); return s; };
  const getAnalyzerSession = (sessionId: string) => { const s = analyzerSessions.get(sessionId); if (!s) err('sessionInvalid', 'Session not found.'); validateSession(nowMs, settingsGeneration, s.createdAtMs, s.settingsGeneration); return s; };
  const getAnySessionMeta = (sessionId: string) => {
    const s = sessions.get(sessionId); if (s) { validateSession(nowMs, settingsGeneration, s.createdAtMs, s.settingsGeneration); return s; }
    const a = analyzerSessions.get(sessionId); if (a) { validateSession(nowMs, settingsGeneration, a.createdAtMs, a.settingsGeneration); return a; }
    err('sessionInvalid', 'Session not found.');
  };

  function storePreview(sessionId: string, itemIds: string[], count: string, bytes: string) {
    previewSeq += 1; const token = `mock-preview-${previewSeq}`;
    previews.set(token, { token, sessionId, createdAtMs: nowMs, used: false, itemIds, settingsGeneration, previewEstimatedBytes: bytes });
    return token;
  }
  function previewForScan(sessionId: string, itemIds: string[], confirmations: { typedConfirmation: string | null } | null): PreviewCleanResult {
    const s = getScanSession(sessionId);
    const selected = previewScanSelection({ session: s, itemIds, allowDeleteUnknown: settingsState.allowDeleteUnknown, confirmations });
    const { breakdown, totalBytes } = scanBreakdown(selected as Array<{ item: ScanItemInfo }>);
    const token = storePreview(sessionId, [...itemIds], String(selected.length), totalBytes.toString());
    return { previewToken: token, breakdown, totalSelectedCount: BigInt(selected.length).toString(), totalSelectedBytes: totalBytes.toString() };
  }
  function previewForAnalyzer(sessionId: string, itemIds: string[], confirmations: { typedConfirmation: string | null } | null): PreviewCleanResult {
    const s = getAnalyzerSession(sessionId);
    const deduped = previewAnalyzerSelection({ session: s, itemIds, allowDeleteUnknown: settingsState.allowDeleteUnknown, confirmations });
    const breakdown = analyzerBreakdown(deduped); const bytes = sumNodeBytes(deduped);
    const token = storePreview(sessionId, deduped.map((n) => n.id), String(deduped.length), bytes);
    return { previewToken: token, breakdown, totalSelectedCount: BigInt(deduped.length).toString(), totalSelectedBytes: bytes };
  }
  function effectiveCleanIds(p: PreviewState) {
    const analyzerSession = analyzerSessions.get(p.sessionId);
    if (!analyzerSession) return [...p.itemIds];
    return dedupeAnalyzerNodes(p.itemIds.map((id) => analyzerSession.nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[]).map((n) => n.id);
  }
  function runExecuteClean(previewToken: string, onProgress?: (e: import('../client').CleanProgressEvent) => void) {
    const p = validatePreviewToken(previews, previewToken, nowMs); getAnySessionMeta(p.sessionId);
    if (p.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
    p.used = true;
    const scanSession = sessions.get(p.sessionId); const analyzerSession = analyzerSessions.get(p.sessionId);
    const ids = effectiveCleanIds(p); let deletedBytes = 0n; let deletedCount = 0n; let skippedCount = 0n; let completed = 0n; const reasons: Record<string, bigint> = {};
    for (const id of ids) {
      const r = cleanOne({ id, scanSession, analyzerSession, allowDeleteUnknown: settingsState.allowDeleteUnknown });
      if (r.ok) { deletedBytes += r.bytes; deletedCount += 1n; } else { skippedCount += 1n; reasons['Skipped: not eligible'] = (reasons['Skipped: not eligible'] ?? 0n) + 1n; }
      completed += 1n; onProgress?.({ previewToken, targetId: r.targetId, completedItems: completed.toString(), totalItems: String(ids.length), bytesFreedSoFar: deletedBytes.toString(), status: r.ok ? 'deleted' : 'Skipped' });
    }
    logSeq += 1; const { log } = buildCleanLog({ itemIds: p.itemIds, deletedCount, deletedBytes, nowMs, logSeq }); logs.unshift(log);
    return { bytesFreed: deletedBytes.toString(), deletedCount: deletedCount.toString(), skippedCount: skippedCount.toString(), skippedReasons: toSkippedReasons(reasons), aborted: false, abortReason: null, logId: log.id };
  }

  return {
    async startScan(tiers, onProgress) {
      scanSeq += 1; const sessionId = `mock-session-${scanSeq}`;
      const { targets, itemsById } = makeScanResults(settingsState, tiers as TierDto[] | null);
      sessions.set(sessionId, { id: sessionId, createdAtMs: nowMs, minAgeHours: settingsState.minAgeHours, targets, itemsById, settingsGeneration });
      await emitScanProgress({ sessionId, targets, onProgress }); return { sessionId };
    },
    async getScanResults(sessionId) { const s = getScanSession(sessionId); return { sessionId: s.id, startedUnixSecs: Math.floor(s.createdAtMs / 1000).toString(), minAgeHours: s.minAgeHours, targets: s.targets }; },
    async startAnalyzer(root, onProgress) {
      analyzerSeq += 1; const sessionId = `mock-analyzer-${analyzerSeq}`; const rootPath = (root && root.trim()) || 'C:\\';
      onProgress?.({ sessionId, scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: rootPath });
      const tree = buildAnalyzerTree(rootPath, settingsState.allowDeleteUnknown);
      const result = { sessionId, rootPath, totalBytes: tree.sizeBytes, totalItems: tree.itemCount, scannedDirs: '42', scannedFiles: String(indexAnalyzerNodes(tree).size), cancelled: false, warnings: [], root: tree };
      onProgress?.({ sessionId, scannedDirs: '42', scannedFiles: result.scannedFiles, scannedBytes: result.totalBytes, currentPath: null });
      analyzerSessions.set(sessionId, { id: sessionId, createdAtMs: nowMs, rootPath, result, nodesById: indexAnalyzerNodes(tree), settingsGeneration }); return result;
    },
    async previewClean(sessionId, itemIds, confirmations) {
      if (sessions.has(sessionId)) return previewForScan(sessionId, itemIds, confirmations ?? null);
      if (analyzerSessions.has(sessionId)) return previewForAnalyzer(sessionId, itemIds, confirmations ?? null);
      err('sessionInvalid', 'Session not found.');
    },
    async executeClean(previewToken, onProgress) { return runExecuteClean(previewToken, onProgress); },
    async explainItem(itemId: string) {
      for (const s of sessions.values()) { const item = s.itemsById.get(itemId); if (item) return { itemId, title: item.targetName, description: item.description, risk: item.risk }; }
      for (const s of analyzerSessions.values()) { const node = s.nodesById.get(itemId); if (node) return { itemId, title: node.name || node.path, description: node.description, risk: node.risk }; }
      err('notFound', 'Item not found.');
    },
    async getSettings() { return { ...settingsState, customPaths: [...settingsState.customPaths] }; },
    async updateSettings(settings) { settingsState = { ...settings, enableElevationTargets: settings.enableElevationTargets ?? false, customPaths: [...(settings.customPaths ?? [])] }; bumpSettings(); return { ...settingsState, customPaths: [...settingsState.customPaths] }; },
    async getElevationStatus() { return { ...elevationStatus }; },
    async relaunchElevated() { if ('error' in relaunchResult) throw relaunchResult.error; return { ...relaunchResult }; },
    async validateCustomPath(path) {
      const trimmed = path.trim();
      if (!trimmed) return { path: trimmed, allowed: false, reason: 'Path is required.' };
      if (!/^[A-Za-z]:\\/.test(trimmed)) return { path: trimmed, allowed: false, reason: 'Path must be absolute.' };
      if (trimmed.toLowerCase().startsWith('c:\\windows')) return { path: trimmed, allowed: false, reason: 'Protected Path (no fallback to parent).' };
      if (trimmed.toLowerCase().includes('\\missing')) return { path: trimmed, allowed: false, reason: 'Path does not exist (no fallback to parent).' };
      if ((trimmed.match(/\\/g) || []).length < 2) return { path: trimmed, allowed: false, reason: 'Path must have depth >= 3 segments.' };
      return { path: trimmed, allowed: true, reason: 'ok' };
    },
    async listLogs() { return logs.map((l) => ({ id: l.id, name: l.name, modifiedUnixSecs: l.modifiedUnixSecs, sizeBytes: l.sizeBytes })); },
    async readLog(logId: string) { const log = logs.find((l) => l.id === logId); if (!log) err('notFound', 'Log not found.'); return { id: log.id, content: log.content }; },
    advanceTimeMs(ms) { nowMs += ms; }, invalidateSettings() { bumpSettings(); }, setElevationStatus(status) { elevationStatus = { ...status }; },
    setRelaunchResult(result) { relaunchResult = 'error' in result ? result : { ...result }; },
    reset() {
      sessions.clear(); analyzerSessions.clear(); previews.clear(); settingsGeneration = 1; settingsState = defaultSettings();
      elevationStatus = { isElevated: false }; relaunchResult = { launched: true }; scanSeq = 0; analyzerSeq = 0; previewSeq = 0; logSeq = 0; nowMs = Date.now();
      logs.length = 0; logs.push(...defaultSeedLogs(nowMs));
    },
  };
}
