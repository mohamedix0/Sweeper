import { formatContractError, type SweeperClient } from '../../api/client';
import type { ContractError, ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { MAX_COMPACT_IDS, preparePreviewSelection } from './previewPerf';
import { isSelectableScanItem } from './scanHelpers';

type Workflow = any;

export function clearTransient(w: Workflow) {
  w.setPreview(null);
  w.setCleanSummary(null);
  w.setCleanProgress(null);
}

function invalidatePreview(w: Workflow) {
  w.invalidatePreviewRequest();
  // The old request's request-ID guard will stop it clearing progress in its
  // finally block, so clear it here or a cancelled Preview leaves a stale
  // "Preparing preview…" status behind.
  w.setPreviewProgress(null);
  clearTransient(w);
}

function addTargetItem(args: { next: Set<string>; item: ScanItemDto; target: TargetScanResultDto; allow: boolean; select: boolean }) {
  if (!isSelectableScanItem({ item: args.item, target: args.target, allowDeleteUnknown: args.allow })) return;
  if (args.select) args.next.add(args.item.id);
  if (!args.select) args.next.delete(args.item.id);
}

export function setTargetSelection(args: { w: Workflow; target: TargetScanResultDto; select: boolean; allow: boolean }) {
  invalidatePreview(args.w);
  args.w.setSelectedIds((prev: Set<string>) => {
    const next = new Set(prev);
    for (const item of args.target.items) addTargetItem({ next, item, target: args.target, allow: args.allow, select: args.select });
    return next;
  });
}

function focusAndInspect(w: Workflow, itemId: string) {
  w.setFocus({ kind: 'item', itemId });
  void w.inspectItem(itemId);
}

export function toggleScanItem(args: { w: Workflow; item: ScanItemDto; target: TargetScanResultDto; allow: boolean }) {
  if (args.w.cleaning) return;
  const selectable = isSelectableScanItem({ item: args.item, target: args.target, allowDeleteUnknown: args.allow });
  if (!selectable) {
    focusAndInspect(args.w, args.item.id);
    return;
  }
  invalidatePreview(args.w);
  args.w.setSelectedIds((prev: Set<string>) => {
    const next = new Set(prev);
    if (next.has(args.item.id)) next.delete(args.item.id); else next.add(args.item.id);
    return next;
  });
  focusAndInspect(args.w, args.item.id);
}

function removeIds(next: Set<string>, ids: string[]) {
  for (const id of ids) next.delete(id);
}

function addIds(next: Set<string>, ids: string[]) {
  for (const id of ids) next.add(id);
}

function applyEligibleToggle(args: { prev: Set<string>; ids: string[]; allSelected: boolean }): Set<string> {
  const next = new Set(args.prev);
  if (args.allSelected) removeIds(next, args.ids);
  if (!args.allSelected) addIds(next, args.ids);
  return next;
}

export function toggleSelectAllEligibleSafe(args: { w: Workflow; allEligibleSelected: boolean; eligibleSafeIds: string[] }) {
  invalidatePreview(args.w);
  args.w.setSelectedIds((prev: Set<string>) => applyEligibleToggle({ prev, ids: args.eligibleSafeIds, allSelected: args.allEligibleSelected }));
}

export function selectAllSafeIds(args: { w: Workflow; selectSafeIds: string[] }) {
  invalidatePreview(args.w);
  args.w.setSelectedIds((prev: Set<string>) => {
    const next = new Set(prev);
    for (const id of args.selectSafeIds) next.add(id);
    return next;
  });
}

export function clearScanSelection(args: { w: Workflow }) {
  invalidatePreview(args.w);
  args.w.setSelectedIds(new Set());
}

function previewStale(args: { w: Workflow; id: number; rev: number; sid: string }): boolean {
  if (args.w.activePreviewRequestRef.current !== args.id) return true;
  if (args.w.previewRevisionRef.current !== args.rev) return true;
  return args.w.sessionId !== args.sid;
}

function markPreviewStart(w: Workflow): { id: number; rev: number } {
  const id = w.activePreviewRequestRef.current + 1;
  w.activePreviewRequestRef.current = id;
  const rev = w.previewRevisionRef.current;
  w.setError(null); w.setInfo(null); w.setPreviewLoading(true); w.setCleanSummary(null); w.setCleanProgress(null);
  return { id, rev };
}

function handlePreviewError(args: { w: Workflow; error: unknown }) {
  const ce = args.error as ContractError | undefined;
  args.w.setError(formatContractError(args.error));
  if (ce?.code === 'sessionInvalid') args.w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
  if (ce?.code === 'previewTokenInvalid') args.w.setPreview(null);
}

function buildConfirmations(args: { requiresConfirmation: boolean; confirmText: string }) {
  if (!args.requiresConfirmation) return null;
  return { typedConfirmation: args.confirmText || null };
}

function handlePreviewCatch(args: { w: Workflow; error: unknown; req: { id: number; rev: number; sid: string } }) {
  if (previewStale({ w: args.w, id: args.req.id, rev: args.req.rev, sid: args.req.sid })) return;
  handlePreviewError({ w: args.w, error: args.error });
}

function finishPreviewLoading(w: Workflow, requestId: number) {
  if (w.activePreviewRequestRef.current === requestId) w.setPreviewLoading(false);
}

function clearPreviewProgress(w: Workflow, requestId: number) {
  if (w.activePreviewRequestRef.current === requestId) w.setPreviewProgress(null);
}

function previewRejected(args: { w: Workflow; selectionSize: number; expired: boolean }): boolean {
  // The Rust decoder refuses descriptors above MAX_COMPACT_IDS; sending one
  // would surface as an unknown item and a fail-closed Preview. Reject early
  // with an actionable message instead of a silent failure.
  if (args.selectionSize > MAX_COMPACT_IDS) {
    args.w.setError(`This selection has ${args.selectionSize.toLocaleString('en-US')} items; Sweeper can Preview at most ${MAX_COMPACT_IDS.toLocaleString('en-US')} items at once. Deselect some items and try again.`);
    return true;
  }
  if (args.expired) {
    args.w.setError('Scan session expired or was invalidated. Start a new Scan.');
    args.w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
    return true;
  }
  return false;
}

export async function runScanPreview(args: { w: Workflow; client: SweeperClient; sel: Set<string>; requiresConfirmation: boolean; sessionId: string; confirmText: string }) {
  const { w } = args;
  if (previewRejected({ w, selectionSize: args.sel.size, expired: w.isSessionExpired })) return;
  const req = markPreviewStart(w);
  const sid = args.sessionId;
  const confirmations = buildConfirmations(args);
  let res: unknown = null;
  try {
    // Chunked, yielding preparation keeps the main thread responsive and gives
    // the view determinate "Preparing preview…" progress for large selections.
    const itemIds = await preparePreviewSelection(
      args.sel,
      (completed, total) => {
        // Request-ID guard: a stale action must not clear/overwrite a newer
        // action's determinate progress state.
        if (w.activePreviewRequestRef.current === req.id) w.setPreviewProgress({ completed, total });
      },
      () => previewStale({ w, id: req.id, rev: req.rev, sid }),
    );
    if (previewStale({ w, id: req.id, rev: req.rev, sid })) {
      finishPreviewLoading(w, req.id);
      return;
    }
    // Encoding is done: stop showing determinate preparation progress before
    // the (possibly slow) Preview IPC, rather than leaving a stale "Preparing
    // preview… N / N" message while the backend builds the Preview.
    clearPreviewProgress(w, req.id);
    res = await args.client.previewClean(sid, itemIds, confirmations);
  } catch (error) {
    handlePreviewCatch({ w, error, req: { id: req.id, rev: req.rev, sid } });
    finishPreviewLoading(w, req.id);
    return;
  } finally {
    clearPreviewProgress(w, req.id);
  }
  if (previewStale({ w, id: req.id, rev: req.rev, sid })) {
    finishPreviewLoading(w, req.id);
    return;
  }
  w.setPreview({ ...(res as object), issuedAtMs: Date.now() });
  finishPreviewLoading(w, req.id);
}

function handleCleanError(args: { w: Workflow; error: unknown }) {
  const ce = args.error as ContractError | undefined;
  args.w.setError(formatContractError(args.error));
  if (ce?.code === 'sessionInvalid') args.w.resetForInvalidSession('Scan session expired or was invalidated. Start a new Scan.');
  if (ce?.code === 'previewTokenInvalid') {
    args.w.setPreview(null);
    args.w.setInfo('Preview token is invalid, expired, or already used. Run Preview again.');
  }
}

const REFRESH_FAILED_NOTICE = 'Clean completed, but the latest Scan results could not be refreshed. Start a new Scan to see current results.';

async function refreshAfterClean(w: Workflow, client: SweeperClient) {
  if (!w.sessionId) return;
  try {
    w.setResults(await client.getScanResults(w.sessionId));
  } catch {
    // Drop pre-Clean items but keep the results shell mounted: the Clean
    // Summary is rendered in its inspector and would otherwise unmount with
    // the whole layout. Surface a non-blocking notice alongside it (#99).
    const current = w.results;
    w.setResults(current ? { ...current, targets: [] } : null);
    w.setInfo(REFRESH_FAILED_NOTICE);
  }
}

export async function runScanClean(args: { w: Workflow; client: SweeperClient }) {
  const { w, client } = args;
  if (!w.preview) return;
  w.setError(null); w.setInfo(null); w.setCleaning(true); w.setCleanProgress(null); w.setCleanSummary(null);
  try {
    const res = await client.executeClean(w.preview.previewToken, (ev: unknown) => w.setCleanProgress(ev));
    w.setCleanSummary(res); w.setPreview(null); w.setSelectedIds(new Set()); w.setConfirmText('');
    w.setInfo('Clean completed. Review the summary and Activity History below.');
    await refreshAfterClean(w, client);
  } catch (error) {
    handleCleanError({ w, error });
  } finally { w.setCleaning(false); }
}

export function createScanActions(ctx: { w: Workflow; client: SweeperClient; allow: boolean; sel: Set<string>; requiresConfirmation: boolean }) {
  void ctx.client; void ctx.sel; void ctx.requiresConfirmation;
  return {
    setTargetSelection: (target: TargetScanResultDto, select: boolean) => setTargetSelection({ w: ctx.w, target, select, allow: ctx.allow }),
    toggleItem: (item: ScanItemDto, target: TargetScanResultDto) => toggleScanItem({ w: ctx.w, item, target, allow: ctx.allow }),
    toggleSelectAllEligibleSafe: (allEligibleSelected: boolean, eligibleSafeIds: string[]) => toggleSelectAllEligibleSafe({ w: ctx.w, allEligibleSelected, eligibleSafeIds }),
    selectAllSafe: (selectSafeIds: string[]) => selectAllSafeIds({ w: ctx.w, selectSafeIds }),
    clearSelection: () => clearScanSelection({ w: ctx.w }),
  };
}
