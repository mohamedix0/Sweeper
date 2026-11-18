import { forwardRef, useEffect, useImperativeHandle, useMemo, useState, type ReactNode } from 'react';
import { type ElevationStatus, type Settings, type SweeperClient } from '../api/client';
import type { ScanItemDto, TargetScanResultDto } from '../contract/bindings';
import { formatBytes } from '../utils/format';
import { TYPED_CONFIRMATION_PHRASE, type PreviewResult } from './cleanShared';
import type { PreviewRiskCalloutSource } from './PreviewRiskCallouts';
import { ScanCleanBar } from './scan/ScanCleanBar';
import { ScanResultsSection, ScanScanningLayout } from './scan/ScanResultsSection';
import { ScanViewControls } from './scan/ScanViewControls';
import { ScanViewHeader } from './scan/ScanViewHeader';
import { ScanNotices, ScanOnboarding, ScanProgressNotice, ScanTerminalStatus } from './scan/ScanViewStatus';
import { EXPLANATION_CONCURRENCY, mapWithConcurrency, planExplanationTasks } from './scan/previewPerf';
import { isSelectableScanItem } from './scan/scanHelpers';
import { useScanDerived } from './scan/useScanDerived';
import { clearScanSelection, runScanClean, runScanPreview, selectAllSafeIds, setTargetSelection, toggleScanItem, toggleSelectAllEligibleSafe } from './scan/useScanViewActions';
import { useScanWorkflow } from './scan/useScanWorkflow';

export type ScanAndPreviewViewHandle = { runScan: () => Promise<void> | void };

type Props = {
  client: SweeperClient;
  settings: Settings | null;
  settingsVersion?: number;
  elevationStatus: ElevationStatus;
  elevationError: string | null;
  onOpenLog: (logId: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
};

function toggleSetId(args: { prev: Set<string>; id: string }): Set<string> {
  const next = new Set(args.prev);
  if (next.has(args.id)) next.delete(args.id);
  else next.add(args.id);
  return next;
}

function focusItemId(focus: ReturnType<typeof useScanWorkflow>['focus']): string | null {
  if (!focus) return null;
  if (focus.kind !== 'item') return null;
  return focus.itemId;
}

function inspectedMatches(args: { inspected: ReturnType<typeof useScanWorkflow>['inspected']; itemId: string | null }): boolean {
  if (!args.itemId) return false;
  if (!args.inspected) return false;
  return args.inspected.itemId === args.itemId;
}

function findItemInTarget(args: { target: TargetScanResultDto; itemId: string }) {
  const item = args.target.items.find((i) => i.id === args.itemId);
  if (!item) return null;
  return { target: args.target, item };
}

function findInspectedCtx(args: { focus: ReturnType<typeof useScanWorkflow>['focus']; inspected: ReturnType<typeof useScanWorkflow>['inspected']; targets: TargetScanResultDto[] }) {
  const itemId = focusItemId(args.focus);
  if (!inspectedMatches({ inspected: args.inspected, itemId })) return null;
  const found = args.targets.map((target) => findItemInTarget({ target, itemId: itemId! })).find((value) => value != null);
  return found ?? null;
}

function cleanProgressLabel(progress: ReturnType<typeof useScanWorkflow>['cleanProgress']): string | null {
  if (!progress) return null;
  return `Progress: ${progress.completedItems} / ${progress.totalItems} items — ${formatBytes({ bytes: progress.bytesFreedSoFar })} freed so far (${progress.status})`;
}

function allSelectableSelected(target: TargetScanResultDto, args: { selectedIds: Set<string>; allow: boolean }): boolean {
  const selectable = target.items.filter((i) => isSelectableScanItem({ item: i, target, allowDeleteUnknown: args.allow }));
  return selectable.every((i) => args.selectedIds.has(i.id));
}

function idleState(args: { scanning: boolean; previewLoading: boolean; cleaning: boolean }): boolean {
  if (args.scanning) return false;
  if (args.previewLoading) return false;
  return !args.cleaning;
}

function canPreview(args: { sessionId: string | null; selected: number; idle: boolean; expired: boolean; confirmed: boolean }): boolean {
  if (args.sessionId == null) return false;
  if (args.selected <= 0) return false;
  if (!args.idle) return false;
  if (args.expired) return false;
  return args.confirmed;
}

function Show(props: { when: boolean; children: ReactNode }) {
  if (!props.when) return null;
  return <>{props.children}</>;
}

function canShowTerminal(args: { scanning: boolean; status: string }): boolean {
  if (args.scanning) return false;
  return Boolean(args.status);
}

function canShowOnboarding(args: { hasResults: boolean; scanning: boolean }): boolean {
  if (args.hasResults) return false;
  return !args.scanning;
}

function showScanningLayout(args: { scanning: boolean; hasResults: boolean }): boolean {
  if (!args.scanning) return false;
  return !args.hasResults;
}

function scanLayoutDisabled(args: { none: boolean; scanning: boolean; cleaning: boolean }): boolean {
  if (args.none) return true;
  if (args.scanning) return true;
  return args.cleaning;
}

function StatusBlocks(props: { w: ReturnType<typeof useScanWorkflow>; d: ReturnType<typeof useScanDerived>; masterIndeterminate: boolean }) {
  const w = props.w;
  const d = props.d;
  const hasResults = !!w.results;
  const disabled = scanLayoutDisabled({ none: d.eligibleSafeIds.length === 0, scanning: w.scanning, cleaning: w.cleaning });
  return (
    <>
      <Show when={w.scanning}><ScanProgressNotice scanStatus={w.scanStatus} scanProgress={w.scanProgress} /></Show>
      <Show when={canShowTerminal({ scanning: w.scanning, status: w.scanStatus })}><ScanTerminalStatus scanStatus={w.scanStatus} /></Show>
      <Show when={canShowOnboarding({ hasResults, scanning: w.scanning })}><ScanOnboarding /></Show>
      <Show when={showScanningLayout({ scanning: w.scanning, hasResults })}><ScanScanningLayout allEligibleSelected={d.allEligibleSelected} masterIndeterminate={props.masterIndeterminate} disabled={disabled} onToggle={() => toggleSelectAllEligibleSafe({ w, allEligibleSelected: d.allEligibleSelected, eligibleSafeIds: d.eligibleSafeIds })} /></Show>
    </>
  );
}

function ResultsBlocks(props: { client: SweeperClient; w: ReturnType<typeof useScanWorkflow>; d: ReturnType<typeof useScanDerived>; focusedGroup: TargetScanResultDto | null; inspectedCtx: ReturnType<typeof findInspectedCtx>; selectionSources: readonly PreviewRiskCalloutSource[]; canPreview: boolean; canClean: boolean; reviewPending: boolean; onOpenLog: Props['onOpenLog']; onOpenSettings: Props['onOpenSettings']; onToggleCollapsed: (id: string) => void; onToggleShowAll: (id: string) => void; onToggleGroupSelect: (t: TargetScanResultDto) => void; cleanProgress: string | null }) {
  if (!props.w.results) return null;
  const { w, d } = props;
  const masterIndeterminate = !d.allEligibleSelected && d.sel.size > 0;
  return (
    <>
      <ScanResultsSection targets={d.targets} visibleTargets={w.visibleTargets} allow={d.allow} minAgeHours={w.effectiveMinAgeHours} selectedIds={d.sel} selectionSources={props.selectionSources} collapsedTargets={w.collapsedTargets} showAllGroups={w.showAllGroups} focus={w.focus} focusedGroup={props.focusedGroup} inspected={w.inspected} inspectedCtx={props.inspectedCtx} cleaning={w.cleaning} scanning={w.scanning} eligibleSafeIdsLength={d.eligibleSafeIds.length} allEligibleSelected={d.allEligibleSelected} masterIndeterminate={masterIndeterminate} requiresConfirmation={d.requiresConfirmation} confirmText={w.confirmText} preview={w.preview} cleanProgressLabel={props.cleanProgress} cleanSummary={w.cleanSummary} onToggleSelectAll={() => toggleSelectAllEligibleSafe({ w, allEligibleSelected: d.allEligibleSelected, eligibleSafeIds: d.eligibleSafeIds })} onFocusGroup={(targetId) => w.setFocus({ kind: 'group', targetId })} onToggleCollapsed={props.onToggleCollapsed} onToggleGroupSelect={props.onToggleGroupSelect} onToggleItem={(item, target) => toggleScanItem({ w, item, target, allow: d.allow })} onToggleShowAll={props.onToggleShowAll} onConfirmText={w.setConfirmText} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      <ScanCleanBar selectedIdsCount={d.sel.size} selectedCategoryCount={d.selectedCategoryCount} selectedBytes={d.selectedBytes} cleaning={w.cleaning} previewLoading={w.previewLoading} hasPreview={!!w.preview} canPreview={props.canPreview} canClean={props.canClean} reviewPending={props.reviewPending} onClear={() => clearScanSelection({ w })} onPreview={() => void runScanPreview({ w, client: props.client, sel: d.sel, requiresConfirmation: d.requiresConfirmation, sessionId: w.sessionId!, confirmText: w.confirmText })} onClean={() => void runScanClean({ w, client: props.client })} />
    </>
  );
}

function focusedGroupFor(focus: ReturnType<typeof useScanWorkflow>['focus'], targets: TargetScanResultDto[]) {
  if (focus?.kind !== 'group') return null;
  return targets.find((t) => t.targetId === focus.targetId) ?? null;
}

type ExplainedItem = Awaited<ReturnType<SweeperClient['explainItem']>>;
type ItemContext = { target: TargetScanResultDto; item: ScanItemDto };
type SelectionSourcesState = {
  sources: readonly PreviewRiskCalloutSource[];
  pending: boolean;
  failedCount: number;
};
type ResolvedSelection = {
  /** Set identity, not a joined key: a new Set is produced on every selection change. */
  key: ReadonlySet<string>;
  preview: PreviewResult;
  sources: readonly PreviewRiskCalloutSource[];
  failedCount: number;
};

const EXPLANATION_UNAVAILABLE = 'Explanation unavailable for this item — showing its known identity and risk only.';
const EMPTY_SELECTION_SOURCES: SelectionSourcesState = { sources: [], pending: false, failedCount: 0 };
const PENDING_SELECTION_SOURCES: SelectionSourcesState = { sources: [], pending: true, failedCount: 0 };

function buildItemContext(args: { targets: TargetScanResultDto[] }): Map<string, ItemContext> {
  const byId = new Map<string, ItemContext>();
  for (const target of args.targets) for (const item of target.items) byId.set(item.id, { target, item });
  return byId;
}

function explainedSource(args: { explained: ExplainedItem; targetId: string | null }): PreviewRiskCalloutSource {
  return {
    itemId: args.explained.itemId,
    title: args.explained.title,
    description: args.explained.description,
    risk: args.explained.risk,
    targetId: args.targetId,
  };
}

function fallbackRisk(args: { targetRisk: TargetScanResultDto['risk'] | undefined }): TargetScanResultDto['risk'] {
  // Preserve the target's known Risk (Safe included); 'Unknown' is only for a
  // genuinely unavailable Risk, never a substitute that invents a warning.
  if (args.targetRisk != null) return args.targetRisk;
  return 'Unknown';
}

function fallbackSource(args: { itemId: string; ctx: ItemContext | undefined }): PreviewRiskCalloutSource {
  return {
    itemId: args.itemId,
    title: args.ctx?.item.path ?? args.itemId,
    description: EXPLANATION_UNAVAILABLE,
    risk: fallbackRisk({ targetRisk: args.ctx?.target.risk }),
    targetId: args.ctx?.target.targetId ?? null,
  };
}

function reportExplanationFailure(args: { itemId: string; error: unknown }): void {
  console.warn(`Sweeper: explanation unavailable for selected item ${args.itemId}.`, args.error);
}

type ResolvedExplanation = { source: PreviewRiskCalloutSource; failed: boolean };

/** Pure render-time gate: explanations must match both the selection and the current Preview generation. */
function derivationPending(args: {
  needs: boolean;
  selection: ReadonlySet<string>;
  preview: PreviewResult | null;
  resolved: ResolvedSelection | null;
}): boolean {
  if (!args.needs) return false;
  return args.resolved == null || args.resolved.key !== args.selection || args.resolved.preview !== args.preview;
}

/** Resolves one item to a callout source, falling back to known identity on failure. */
async function resolveItemExplanation(args: { client: SweeperClient; itemContext: Map<string, ItemContext>; itemId: string }): Promise<ResolvedExplanation> {
  const ctx = args.itemContext.get(args.itemId);
  try {
    const explained = await args.client.explainItem(args.itemId);
    return { source: explainedSource({ explained, targetId: ctx?.target.targetId ?? null }), failed: false };
  } catch (error) {
    reportExplanationFailure({ itemId: args.itemId, error });
    return { source: fallbackSource({ itemId: args.itemId, ctx }), failed: true };
  }
}

/**
 * Resolve a bounded summary of explanations; a stale generation is ignored on
 * arrival. Small selections keep one IPC per item (preserving per-item failure
 * notices); large selections resolve one representative per Target, memoised
 * by Target for this Preview generation, with capped concurrency.
 */
function resolveExplanations(args: {
  client: SweeperClient;
  selection: ReadonlySet<string>;
  preview: PreviewResult;
  allTargets: TargetScanResultDto[];
  setResolved: (value: ResolvedSelection) => void;
  isCurrent: () => boolean;
}): void {
  if (args.selection.size === 0) return;
  const itemContext = buildItemContext({ targets: args.allTargets });
  const tasks = planExplanationTasks({ selected: args.selection, targetIdFor: (id) => itemContext.get(id)?.target.targetId });
  const memo = new Map<string, Promise<ResolvedExplanation>>();
  const run = mapWithConcurrency(tasks, EXPLANATION_CONCURRENCY, (task) => {
    const cached = memo.get(task.memoKey);
    if (cached) return cached;
    const pending = resolveItemExplanation({ client: args.client, itemContext, itemId: task.itemId });
    memo.set(task.memoKey, pending);
    return pending;
  });
  void run.then((results) => {
    if (!args.isCurrent()) return;
    args.setResolved({
      key: args.selection,
      preview: args.preview,
      sources: results.map((result) => result.source),
      failedCount: results.filter((result) => result.failed).length,
    });
  });
}

/**
 * Resolve Knowledge Base explanations for the current selection. Explanations
 * are keyed by Set identity AND the current Preview, so reopening a Preview
 * with the same selection re-gates Clean until the new explanations resolve.
 */
function useSelectionSources(args: {
  client: SweeperClient;
  selection: ReadonlySet<string>;
  allTargets: TargetScanResultDto[];
  preview: PreviewResult | null;
}): SelectionSourcesState {
  const { client, selection, allTargets, preview } = args;
  const [resolved, setResolved] = useState<ResolvedSelection | null>(null);
  const needs = preview != null && selection.size > 0;

  useEffect(() => {
    let cancelled = false;
    if (preview != null && needs) {
      resolveExplanations({ client, selection, preview, allTargets, setResolved, isCurrent: () => !cancelled });
    }
    return () => { cancelled = true; };
  }, [client, selection, allTargets, preview, needs]);

  if (!needs) return EMPTY_SELECTION_SOURCES;
  if (derivationPending({ needs, selection, preview, resolved })) return PENDING_SELECTION_SOURCES;
  return { sources: resolved!.sources, pending: false, failedCount: resolved!.failedCount };
}

function explanationWarning(args: { failedCount: number }): string | null {
  if (args.failedCount <= 0) return null;
  return `Explanation unavailable for ${args.failedCount} selected item(s); showing known identity and risk only.`;
}

function collapseAllTargets(args: { targets: TargetScanResultDto[]; hasCollapsed: boolean }): Set<string> {
  if (args.targets.length === 0) return new Set<string>();
  if (args.hasCollapsed) return new Set<string>();
  return new Set(args.targets.map((t) => t.targetId));
}

function isTypedConfirmSatisfied(args: { requiresConfirm: boolean; typedDelete: boolean }): boolean {
  if (!args.requiresConfirm) return true;
  return args.typedDelete;
}

function canCleanWithPreview(args: { hasPreview: boolean; idle: boolean }): boolean {
  if (!args.hasPreview) return false;
  return args.idle;
}

function anyTargetsCollapsed(args: { hasTargets: boolean; hasCollapsed: boolean }): boolean {
  if (!args.hasTargets) return false;
  return args.hasCollapsed;
}

function masterCheckboxIndeterminate(args: { allSelected: boolean; hasSelection: boolean }): boolean {
  if (args.allSelected) return false;
  return args.hasSelection;
}

export const ScanAndPreviewView = forwardRef<ScanAndPreviewViewHandle, Props>(function ScanAndPreviewView(props, ref) {
  const { client, settings, settingsVersion, elevationError, onOpenLog, onOpenSettings } = props;
  const w = useScanWorkflow({ client, settings, settingsVersion });
  const d = useScanDerived(w, settingsVersion);
  const inspectedCtx = useMemo(() => findInspectedCtx({ focus: w.focus, inspected: w.inspected, targets: d.targets }), [w.focus, w.inspected, d.targets]);
  const focusedGroup = useMemo(() => focusedGroupFor(w.focus, d.targets), [w.focus, d.targets]);
  const selection = d.sel;
  const allTargets = useMemo(() => w.results?.targets ?? [], [w.results]);
  const previewActive = w.preview != null;
  const selectionState = useSelectionSources({ client, selection, allTargets, preview: w.preview });
  const progress = cleanProgressLabel(w.cleanProgress);
  const prep = w.previewProgress;
  useImperativeHandle(ref, () => ({ runScan: w.runScan }), [w.runScan]);
  const onToggleCollapsed = (targetId: string) => w.setCollapsedTargets((prev) => toggleSetId({ prev, id: targetId }));
  const onToggleShowAll = (targetId: string) => w.setShowAllGroups((prev) => toggleSetId({ prev, id: targetId }));
  const hasTargets = d.targets.length > 0;
  const hasCollapsed = w.collapsedTargets.size > 0;
  const onToggleCollapseAll = () => w.setCollapsedTargets(collapseAllTargets({ targets: d.targets, hasCollapsed }));
  const onToggleGroupSelect = (target: TargetScanResultDto) => setTargetSelection({ w, target, select: !allSelectableSelected(target, { selectedIds: d.sel, allow: d.allow }), allow: d.allow });
  const typedDelete = w.confirmText === TYPED_CONFIRMATION_PHRASE;
  const confirmed = isTypedConfirmSatisfied({ requiresConfirm: d.requiresConfirmation, typedDelete });
  const idle = idleState({ scanning: w.scanning, previewLoading: w.previewLoading, cleaning: w.cleaning });
  const previewOk = canPreview({ sessionId: w.sessionId, selected: d.sel.size, idle, expired: w.isSessionExpired, confirmed });
  const cleanOk = canCleanWithPreview({ hasPreview: previewActive, idle }) && !selectionState.pending;
  const anyCollapsed = anyTargetsCollapsed({ hasTargets, hasCollapsed });
  const masterIndeterminate = masterCheckboxIndeterminate({ allSelected: d.allEligibleSelected, hasSelection: d.sel.size > 0 });
  return (
    <div className="view-stack view-stack-fill">
      <ScanViewHeader hasResults={!!w.results} heroReclaimable={d.heroReclaimable} tier1={w.tier1} tier2={w.tier2} scanning={w.scanning} cleaning={w.cleaning} settings={settings} settingsStale={d.settingsStale} hasActiveScope={w.hasActiveScanScope} onTier1={w.setTier1} onTier2={w.setTier2} onRunScan={() => void w.runScan()} onOpenSettings={onOpenSettings} />
      <ScanNotices elevationError={elevationError} relaunchInfo={w.relaunchInfo} relaunchError={w.relaunchError} error={w.error} info={w.info} explanationWarning={explanationWarning({ failedCount: selectionState.failedCount })} />
      <ScanViewControls toolbar={w.toolbar} targetsCount={d.targets.length} selectableCount={d.selectableCount} skippedCount={d.skippedCount} hasResults={!!w.results} cleaning={w.cleaning} anyCollapsed={anyCollapsed} isFilterActive={d.isFilterActive} selectSafeIdsLength={d.selectSafeIds.length} allSelectSafeSelected={d.allSelectSafeSelected} onToolbar={w.setToolbar} onToggleCollapseAll={onToggleCollapseAll} onSelectAllSafe={() => selectAllSafeIds({ w, selectSafeIds: d.selectSafeIds })} />
      <StatusBlocks w={w} d={d} masterIndeterminate={masterIndeterminate} />
      {prep != null && (
        <div className="preview-preparing" role="status" aria-live="polite" style={{ display: 'flex', alignItems: 'center', gap: '0.75rem' }}>
          <span>Preparing preview… {prep.completed} / {prep.total} items</span>
          <progress value={prep.completed} max={prep.total} />
        </div>
      )}
      <ResultsBlocks client={client} w={w} d={d} focusedGroup={focusedGroup} inspectedCtx={inspectedCtx} selectionSources={selectionState.sources} canPreview={previewOk} canClean={cleanOk} reviewPending={selectionState.pending} onOpenLog={onOpenLog} onOpenSettings={onOpenSettings} onToggleCollapsed={onToggleCollapsed} onToggleShowAll={onToggleShowAll} onToggleGroupSelect={onToggleGroupSelect} cleanProgress={progress} />
    </div>
  );
});
