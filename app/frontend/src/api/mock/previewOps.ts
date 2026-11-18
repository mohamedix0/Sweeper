import type { AnalyzerNodeDto, PreviewCleanResult } from '../client';
import { expandPreviewSelectionIds } from '../../components/scan/previewPerf';
import { analyzerBreakdown, dedupeAnalyzerNodes, err, isUnknownBlocked, scanBreakdown, sumNodeBytes } from './helpers';
import { getValidSessionState } from './scanAnalyzerOps';
import type { AnalyzerSessionState, MockCtx, ScanSessionState } from './types';
import type { ScanItemInfo } from './data';

type Confirmations = { typedConfirmation: string | null } | null | undefined;
type PreviewRequest = { ctx: MockCtx; sessionId: string; itemIds: string[]; confirmations: Confirmations };
type PreviewBuild = { sessionId: string; storedIds: string[]; count: number; bytes: string; breakdown: PreviewCleanResult['breakdown'] };
type ScanCandidate = { kind: 'scan'; item: ScanItemInfo };
type AnalyzerCandidate = { kind: 'analyzer'; node: AnalyzerNodeDto };
type Candidate = ScanCandidate | AnalyzerCandidate;

function typedConfirmation(input: { confirmations: Confirmations }): string | null {
  if (!input.confirmations) return null;
  return input.confirmations.typedConfirmation;
}

function requireTyped(input: { confirmations: Confirmations }) {
  if (typedConfirmation(input) === 'DELETE') return;
  err({ code: 'invalidInput', reason: 'Typed confirmation required: enter DELETE to continue.' });
}

function ensureSelectionPresent(input: { itemIds: string[] }) {
  if (input.itemIds.length > 0) return;
  err({ code: 'invalidInput', reason: 'No items selected.' });
}

function scanCandidateEligible(input: { item: ScanItemInfo; allowDeleteUnknown: boolean }): boolean {
  if (input.item.risk === 'Protected') return false;
  if (!input.item.ageEligible) return false;
  return !isUnknownBlocked({ risk: input.item.risk, allowDeleteUnknown: input.allowDeleteUnknown });
}

function analyzerCandidateEligible(input: { node: AnalyzerNodeDto; allowDeleteUnknown: boolean }): boolean {
  if (!input.node.selectable) return false;
  if (input.node.displayOnly) return false;
  if (input.node.risk === 'Protected') return false;
  if (input.node.isLinkPoint) return false;
  if (input.node.isCloudPlaceholder) return false;
  if (input.node.risk !== 'Unknown') return true;
  return input.allowDeleteUnknown;
}

function candidateNeedsConfirm(candidate: Candidate): boolean {
  if (candidate.kind === 'scan') return candidate.item.risk === 'Unknown' || candidate.item.targetId.startsWith('custom-path');
  return candidate.node.risk === 'Unknown' || !candidate.node.targetId;
}

function assertScanCandidateAllowed(input: { item: ScanItemInfo; allowDeleteUnknown: boolean }) {
  if (scanCandidateEligible({ item: input.item, allowDeleteUnknown: input.allowDeleteUnknown })) return;
  if (input.item.risk === 'Protected') err({ code: 'invalidInput', reason: 'Protected items cannot be selected.' });
  if (!input.item.ageEligible) err({ code: 'invalidInput', reason: 'Ineligible items cannot be selected.' });
  err({ code: 'invalidInput', reason: 'Unknown items are gated by allow_delete_unknown.' });
}

function assertAnalyzerCandidateAllowed(input: { node: AnalyzerNodeDto; allowDeleteUnknown: boolean }) {
  if (isUnknownBlocked({ risk: input.node.risk, allowDeleteUnknown: input.allowDeleteUnknown })) {
    err({ code: 'invalidInput', reason: 'Unknown items are gated by allow_delete_unknown.' });
  }
  if (analyzerCandidateEligible({ node: input.node, allowDeleteUnknown: true })) return;
  err({ code: 'invalidInput', reason: 'Selected analyzer node is not eligible for Review & Clean.' });
}

function assertCandidateAllowed(input: { candidate: Candidate; allowDeleteUnknown: boolean }) {
  if (input.candidate.kind === 'scan') {
    assertScanCandidateAllowed({ item: input.candidate.item, allowDeleteUnknown: input.allowDeleteUnknown });
    return;
  }
  assertAnalyzerCandidateAllowed({ node: input.candidate.node, allowDeleteUnknown: input.allowDeleteUnknown });
}

function findCandidate(input: { session: ScanSessionState | AnalyzerSessionState; itemId: string }): Candidate | null {
  if ('itemsById' in input.session) {
    const item = input.session.itemsById.get(input.itemId);
    return item ? { kind: 'scan', item } : null;
  }
  const node = input.session.nodesById.get(input.itemId);
  return node ? { kind: 'analyzer', node } : null;
}

type SelectionState = { selected: Candidate[]; unknownCount: number; needsConfirm: boolean };

function evaluateSelectedCandidate(input: { session: ScanSessionState | AnalyzerSessionState; itemId: string; selection: SelectionState; allowDeleteUnknown: boolean }) {
  const candidate = findCandidate({ session: input.session, itemId: input.itemId });
  if (!candidate) {
    input.selection.unknownCount += 1;
    return;
  }
  assertCandidateAllowed({ candidate, allowDeleteUnknown: input.allowDeleteUnknown });
  input.selection.selected.push(candidate);
  if (!candidateNeedsConfirm(candidate)) return;
  input.selection.needsConfirm = true;
}

function selectCandidates(input: { session: ScanSessionState | AnalyzerSessionState; itemIds: string[]; confirmations: Confirmations; allowDeleteUnknown: boolean }): Candidate[] {
  ensureSelectionPresent({ itemIds: input.itemIds });
  const selection: SelectionState = { selected: [], unknownCount: 0, needsConfirm: false };
  for (const itemId of input.itemIds) {
    evaluateSelectedCandidate({ session: input.session, itemId, selection, allowDeleteUnknown: input.allowDeleteUnknown });
  }
  if (selection.unknownCount > 0) err({ code: 'unknownItem', count: String(selection.unknownCount) });
  if (selection.needsConfirm) requireTyped({ confirmations: input.confirmations });
  return selection.selected;
}

function buildSelection(input: { session: ScanSessionState | AnalyzerSessionState; itemIds: string[]; confirmations: Confirmations; allowDeleteUnknown: boolean }): PreviewBuild {
  const selected = selectCandidates({ session: input.session, itemIds: input.itemIds, confirmations: input.confirmations, allowDeleteUnknown: input.allowDeleteUnknown });
  if ('itemsById' in input.session) {
    const items = selected.flatMap((candidate) => (candidate.kind === 'scan' ? [candidate.item] : []));
    const { breakdown, totalBytes } = scanBreakdown(items.map((item) => ({ item })));
    return { sessionId: input.session.id, storedIds: [...input.itemIds], count: items.length, bytes: totalBytes.toString(), breakdown };
  }
  const nodes = dedupeAnalyzerNodes(selected.flatMap((candidate) => (candidate.kind === 'analyzer' ? [candidate.node] : [])));
  return { sessionId: input.session.id, storedIds: nodes.map((node) => node.id), count: nodes.length, bytes: sumNodeBytes(nodes), breakdown: analyzerBreakdown(nodes) };
}

function buildPreview(input: PreviewRequest): PreviewBuild {
  const allowDeleteUnknown = input.ctx.mutable.settingsState.allowDeleteUnknown;
  const itemIds = expandPreviewSelectionIds(input.itemIds);
  if (input.ctx.sessions.has(input.sessionId)) {
    const session = getValidSessionState({ ctx: input.ctx, sessions: input.ctx.sessions, sessionId: input.sessionId });
    return buildSelection({ session, itemIds, confirmations: input.confirmations, allowDeleteUnknown });
  }
  if (input.ctx.analyzerSessions.has(input.sessionId)) {
    const session = getValidSessionState({ ctx: input.ctx, sessions: input.ctx.analyzerSessions, sessionId: input.sessionId });
    return buildSelection({ session, itemIds, confirmations: input.confirmations, allowDeleteUnknown });
  }
  err({ code: 'sessionInvalid', reason: 'Session not found.' });
}

function storePreview(input: { ctx: MockCtx; build: PreviewBuild }): string {
  input.ctx.previewSeq.value += 1;
  const token = 'mock-preview-' + String(input.ctx.previewSeq.value);
  input.ctx.previews.set(token, {
    token,
    sessionId: input.build.sessionId,
    createdAtMs: input.ctx.state.nowMs,
    used: false,
    itemIds: input.build.storedIds,
    settingsGeneration: input.ctx.state.settingsGeneration,
    previewEstimatedBytes: input.build.bytes,
  });
  return token;
}

function issuePreview(input: { ctx: MockCtx; build: PreviewBuild }): PreviewCleanResult {
  const token = storePreview(input);
  return {
    previewToken: token,
    breakdown: input.build.breakdown,
    totalSelectedCount: BigInt(input.build.count).toString(),
    totalSelectedBytes: input.build.bytes,
  };
}

export function buildPreviewApi(ctx: MockCtx) {
  return {
    async previewClean(sessionId: string, itemIds: string[], confirmations?: { typedConfirmation: string | null } | null) {
      return issuePreview({ ctx, build: buildPreview({ ctx, sessionId, itemIds, confirmations: confirmations ?? null }) });
    },
  };
}
