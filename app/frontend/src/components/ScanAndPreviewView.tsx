import { forwardRef, useEffect, useImperativeHandle, useMemo, useState, type ReactNode } from 'react';
import { type ElevationStatus, type Settings, type SweeperClient } from '../api/client';
import type { TargetScanResultDto } from '../contract/bindings';
import { formatBytes } from '../utils/format';
import { TYPED_CONFIRMATION_PHRASE } from './cleanShared';
import type { PreviewRiskCalloutSource } from './PreviewRiskCallouts';
import { ScanCleanBar } from './scan/ScanCleanBar';
import { ScanResultsSection, ScanScanningLayout } from './scan/ScanResultsSection';
import { ScanViewControls } from './scan/ScanViewControls';
import { ScanViewHeader } from './scan/ScanViewHeader';
import { ScanNotices, ScanOnboarding, ScanProgressNotice, ScanTerminalStatus } from './scan/ScanViewStatus';
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

function toggleSetId(prev: Set<string>, id: string): Set<string> {
  const next = new Set(prev);
  if (next.has(id)) next.delete(id);
  else next.add(id);
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

function findItemInTarget(target: TargetScanResultDto, itemId: string) {
  const item = target.items.find((i) => i.id === itemId);
  if (!item) return null;
  return { target, item };
}

function findInspectedCtx(args: { focus: ReturnType<typeof useScanWorkflow>['focus']; inspected: ReturnType<typeof useScanWorkflow>['inspected']; targets: TargetScanResultDto[] }) {
  const itemId = focusItemId(args.focus);
  if (!inspectedMatches({ inspected: args.inspected, itemId })) return null;
  const found = args.targets.map((target) => findItemInTarget(target, itemId!)).find((value) => value != null);
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

function ResultsBlocks(props: { client: SweeperClient; w: ReturnType<typeof useScanWorkflow>; d: ReturnType<typeof useScanDerived>; focusedGroup: TargetScanResultDto | null; inspectedCtx: ReturnType<typeof findInspectedCtx>; selectionSources: readonly PreviewRiskCalloutSource[]; canPreview: boolean; canClean: boolean; onOpenLog: Props['onOpenLog']; onOpenSettings: Props['onOpenSettings']; onToggleCollapsed: (id: string) => void; onToggleShowAll: (id: string) => void; onToggleGroupSelect: (t: TargetScanResultDto) => void; cleanProgress: string | null }) {
  if (!props.w.results) return null;
  const { w, d } = props;
  const masterIndeterminate = !d.allEligibleSelected && d.sel.size > 0;
  return (
    <>
      <ScanResultsSection targets={d.targets} visibleTargets={w.visibleTargets} allow={d.allow} minAgeHours={w.effectiveMinAgeHours} selectedIds={d.sel} selectionSources={props.selectionSources} collapsedTargets={w.collapsedTargets} showAllGroups={w.showAllGroups} focus={w.focus} focusedGroup={props.focusedGroup} inspected={w.inspected} inspectedCtx={props.inspectedCtx} cleaning={w.cleaning} scanning={w.scanning} eligibleSafeIdsLength={d.eligibleSafeIds.length} allEligibleSelected={d.allEligibleSelected} masterIndeterminate={masterIndeterminate} requiresConfirmation={d.requiresConfirmation} confirmText={w.confirmText} preview={w.preview} cleanProgressLabel={props.cleanProgress} cleanSummary={w.cleanSummary} onToggleSelectAll={() => toggleSelectAllEligibleSafe({ w, allEligibleSelected: d.allEligibleSelected, eligibleSafeIds: d.eligibleSafeIds })} onFocusGroup={(targetId) => w.setFocus({ kind: 'group', targetId })} onToggleCollapsed={props.onToggleCollapsed} onToggleGroupSelect={props.onToggleGroupSelect} onToggleItem={(item, target) => toggleScanItem({ w, item, target, allow: d.allow })} onToggleShowAll={props.onToggleShowAll} onConfirmText={w.setConfirmText} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      <ScanCleanBar selectedIdsCount={d.sel.size} selectedCategoryCount={d.selectedCategoryCount} selectedBytes={d.selectedBytes} cleaning={w.cleaning} previewLoading={w.previewLoading} hasPreview={!!w.preview} canPreview={props.canPreview} canClean={props.canClean} onClear={() => clearScanSelection({ w })} onPreview={() => void runScanPreview({ w, client: props.client, sel: d.sel, requiresConfirmation: d.requiresConfirmation, sessionId: w.sessionId!, confirmText: w.confirmText })} onClean={() => void runScanClean({ w, client: props.client })} />
    </>
  );
}

function focusedGroupFor(focus: ReturnType<typeof useScanWorkflow>['focus'], targets: TargetScanResultDto[]) {
  if (focus?.kind !== 'group') return null;
  return targets.find((t) => t.targetId === focus.targetId) ?? null;
}

export const ScanAndPreviewView = forwardRef<ScanAndPreviewViewHandle, Props>(function ScanAndPreviewView(props, ref) {
  const { client, settings, settingsVersion, elevationError, onOpenLog, onOpenSettings } = props;
  const w = useScanWorkflow({ client, settings, settingsVersion });
  const d = useScanDerived(w, settingsVersion);
  const inspectedCtx = useMemo(() => findInspectedCtx({ focus: w.focus, inspected: w.inspected, targets: d.targets }), [w.focus, w.inspected, d.targets]);
  const focusedGroup = useMemo(() => focusedGroupFor(w.focus, d.targets), [w.focus, d.targets]);
  const [selectionSources, setSelectionSources] = useState<readonly PreviewRiskCalloutSource[]>([]);
  const selectedKey = useMemo(() => [...d.sel].sort().join('\u0000'), [d.sel]);
  const allTargets = useMemo(() => w.results?.targets ?? [], [w.results]);
  const previewActive = w.preview != null;
  useEffect(() => {
    let cancelled = false;
    const ids = selectedKey === '' ? [] : selectedKey.split('\u0000');
    if (!previewActive || ids.length === 0) {
      setSelectionSources([]);
      return;
    }
    const targetIdByItem = new Map<string, string>();
    for (const t of allTargets) for (const i of t.items) targetIdByItem.set(i.id, t.targetId);
    void Promise.all(ids.map(async (itemId) => {
      try {
        return await client.explainItem(itemId);
      } catch {
        return null;
      }
    })).then((explained) => {
      if (cancelled) return;
      const sources: PreviewRiskCalloutSource[] = [];
      for (const e of explained) {
        if (!e) continue;
        sources.push({
          itemId: e.itemId,
          title: e.title,
          description: e.description,
          risk: e.risk,
          targetId: targetIdByItem.get(e.itemId) ?? null,
        });
      }
      setSelectionSources(sources);
    });
    return () => { cancelled = true; };
  }, [client, selectedKey, allTargets, previewActive]);
  const progress = cleanProgressLabel(w.cleanProgress);
  useImperativeHandle(ref, () => ({ runScan: w.runScan }), [w.runScan]);
  const onToggleCollapsed = (targetId: string) => w.setCollapsedTargets((prev) => toggleSetId(prev, targetId));
  const onToggleShowAll = (targetId: string) => w.setShowAllGroups((prev) => toggleSetId(prev, targetId));
  const hasTargets = d.targets.length > 0;
  const hasCollapsed = w.collapsedTargets.size > 0;
  const collapseAllState = () => {
    if (!hasTargets) return new Set<string>();
    if (hasCollapsed) return new Set<string>();
    return new Set(d.targets.map((t) => t.targetId));
  };
  const onToggleCollapseAll = () => w.setCollapsedTargets(collapseAllState());
  const onToggleGroupSelect = (target: TargetScanResultDto) => setTargetSelection({ w, target, select: !allSelectableSelected(target, { selectedIds: d.sel, allow: d.allow }), allow: d.allow });
  const requiresConfirm = d.requiresConfirmation;
  const typedDelete = w.confirmText === TYPED_CONFIRMATION_PHRASE;
  let confirmed = true;
  if (requiresConfirm) confirmed = typedDelete;
  const idle = idleState({ scanning: w.scanning, previewLoading: w.previewLoading, cleaning: w.cleaning });
  const previewOk = canPreview({ sessionId: w.sessionId, selected: d.sel.size, idle, expired: w.isSessionExpired, confirmed });
  const hasPreview = w.preview != null;
  let cleanOk = false;
  if (hasPreview) cleanOk = idle;
  let anyCollapsed = false;
  if (hasTargets) anyCollapsed = hasCollapsed;
  const hasSelectedCount = d.sel.size > 0;
  let masterIndeterminate = hasSelectedCount;
  if (d.allEligibleSelected) masterIndeterminate = false;
  return (
    <div className="view-stack view-stack-fill">
      <ScanViewHeader hasResults={!!w.results} heroReclaimable={d.heroReclaimable} tier1={w.tier1} tier2={w.tier2} scanning={w.scanning} cleaning={w.cleaning} settings={settings} settingsStale={d.settingsStale} hasActiveScope={w.hasActiveScanScope} onTier1={w.setTier1} onTier2={w.setTier2} onRunScan={() => void w.runScan()} onOpenSettings={onOpenSettings} />
      <ScanNotices elevationError={elevationError} relaunchInfo={w.relaunchInfo} relaunchError={w.relaunchError} error={w.error} info={w.info} />
      <ScanViewControls toolbar={w.toolbar} targetsCount={d.targets.length} selectableCount={d.selectableCount} skippedCount={d.skippedCount} hasResults={!!w.results} cleaning={w.cleaning} anyCollapsed={anyCollapsed} isFilterActive={d.isFilterActive} selectSafeIdsLength={d.selectSafeIds.length} allSelectSafeSelected={d.allSelectSafeSelected} onToolbar={w.setToolbar} onToggleCollapseAll={onToggleCollapseAll} onSelectAllSafe={() => selectAllSafeIds({ w, selectSafeIds: d.selectSafeIds })} />
      <StatusBlocks w={w} d={d} masterIndeterminate={masterIndeterminate} />
      <ResultsBlocks client={client} w={w} d={d} focusedGroup={focusedGroup} inspectedCtx={inspectedCtx} selectionSources={selectionSources} canPreview={previewOk} canClean={cleanOk} onOpenLog={onOpenLog} onOpenSettings={onOpenSettings} onToggleCollapsed={onToggleCollapsed} onToggleShowAll={onToggleShowAll} onToggleGroupSelect={onToggleGroupSelect} cleanProgress={progress} />
    </div>
  );
});