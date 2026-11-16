import type { useAnalyzerViewModel } from './useAnalyzerViewModel';
import type { useAnalyzerWorkflow } from './useAnalyzerWorkflow';

type W = ReturnType<typeof useAnalyzerWorkflow>;
type VM = ReturnType<typeof useAnalyzerViewModel>;

type OpenSettings = ((focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void) | undefined;
type Handlers = {
  onInspect: (args: { id: string }) => void;
  onShowMore: (args: { parentId: string }) => void;
  onDrill: (args: { path: string }) => void;
};

type BuildArgs = { w: W; vm: VM; onOpenLog: (id: string) => void; onOpenSettings: OpenSettings; handlers: Handlers };

function workflowSlice(input: BuildArgs) {
  const { w, vm } = input;
  return {
    rows: w.flatRows,
    rootTotalBytes: w.rootTotalBytes,
    expanded: w.expanded,
    selectedIds: w.selectedIds,
    inspectedId: w.inspected?.itemId ?? null,
    allowDeleteUnknown: w.allowDeleteUnknown,
    cleaning: w.cleaning,
    analyzing: w.analyzing,
    previewLoading: w.previewLoading,
    statusFilter: w.statusFilter,
    statusCounts: w.statusCounts,
    query: w.query,
    sort: w.sort,
    allVisibleExpanded: vm.allVisibleExpanded,
    visibleSelectableCount: vm.visibleSelectableIds.length,
    visibleSelectedCount: vm.visibleSelectedCount,
    allVisibleSelectableSelected: vm.allVisibleSelectableSelected,
    inspected: w.inspected,
    inspectedNode: vm.inspectedNode,
  };
}

function selectionSlice(input: BuildArgs) {
  const { w, vm } = input;
  return {
    requiresConfirmation: w.requiresConfirmation,
    confirmText: w.confirmText,
    preview: w.preview,
    cleanProgressLabel: vm.cleanProgressLabel,
    cleanSummary: w.cleanSummary,
    selectedCount: w.selectedIds.size,
    selectedBytes: w.selectedBytes,
    hasSelectedRisky: vm.hasSelectedRisky,
    canPreview: vm.canPreview,
    canClean: vm.canClean,
  };
}

function actionSlice(input: BuildArgs) {
  const { w, vm } = input;
  return {
    onStatus: w.setStatusFilter,
    onQuery: w.setQuery,
    onSort: w.setSort,
    onSelectSafe: w.selectSafeAnalyzer,
    onToggleExpandCollapse: () => w.toggleExpandCollapseAll(vm.visibleExpandableIds),
    onToggleSelectAll: vm.toggleSelectAllVisible,
    onToggleExpand: w.toggleExpand,
    onToggleSelect: w.toggleSelect,
    onInspect: input.handlers.onInspect,
    onShowMore: input.handlers.onShowMore,
    onDrill: input.handlers.onDrill,
    onConfirmText: w.setConfirmText,
    onOpenLog: input.onOpenLog,
    onOpenSettings: input.onOpenSettings,
    onClear: vm.clearSelection,
    onPreview: () => void w.runPreview(),
    onClean: () => void w.runClean(),
  };
}

export function buildAnalyzerResultsProps(input: BuildArgs) {
  return {
    ...workflowSlice(input),
    ...selectionSlice(input),
    ...actionSlice(input),
  };
}
