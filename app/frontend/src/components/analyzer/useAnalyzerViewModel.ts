import { normalizeDisplayPath } from '../../utils/format';
import { buildAllocSegments } from './analyzerAlloc';
import { isAncestorAnalyzerPath } from './analyzerHelpers';
import { canPreviewOf, cleanProgressLabelOf, collectExpandableIds, visibleSelectableIdsOf } from './analyzerViewUtils';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { useAnalyzerWorkflow } from './useAnalyzerWorkflow';

type Workflow = ReturnType<typeof useAnalyzerWorkflow>;

function computeDisplayPath(w: Workflow): string {
  let basePath = w.analyzedPath;
  if (!basePath) basePath = w.rootPath;
  const trimmedPath = (basePath || '').trim();
  return normalizeDisplayPath({ p: trimmedPath || 'C:\\' });
}

function computeInspectedNode(w: Workflow) {
  if (!w.inspected) return null;
  return w.nodesById.get(w.inspected.itemId) ?? null;
}

function computeCanClean(w: Workflow): boolean {
  let canClean = Boolean(w.preview);
  if (w.analyzing) canClean = false;
  if (w.previewLoading) canClean = false;
  if (w.cleaning) canClean = false;
  return canClean;
}

function computeProgressPath(w: Workflow): string | null {
  if (!w.progress?.currentPath) return null;
  return normalizeDisplayPath({ p: w.progress.currentPath });
}

export function useAnalyzerViewModel(args: { w: Workflow }) {
  const w = args.w;
  const allocSegments = buildAllocSegments(w.result, w.rootTotalBytes);
  const displayPath = computeDisplayPath(w);
  const visibleExpandableIds = collectExpandableIds({ filteredRoot: w.filteredRoot as never });
  const allVisibleExpanded = visibleExpandableIds.length > 0 && visibleExpandableIds.every((id) => w.expanded.has(id));
  const visibleSelectableIds = visibleSelectableIdsOf({ rows: w.flatRows as never, allowDeleteUnknown: w.allowDeleteUnknown });
  const allVisibleSelectableSelected = visibleSelectableIds.length > 0 && visibleSelectableIds.every((id) => w.selectedIds.has(id));
  const visibleSelectedCount = visibleSelectableIds.filter((id) => w.selectedIds.has(id)).length;
  const hasResult = Boolean(w.result);
  const hasSelectedRisky = w.selectedNodes.some((n) => n.risk === 'Caution' || n.risk === 'Unknown');
  const cleanProgressLabel = cleanProgressLabelOf(w.cleanProgress);
  const inspectedNode = computeInspectedNode(w);
  const canPreview = canPreviewOf({ sessionId: w.sessionId, selectedSize: w.selectedIds.size, analyzing: w.analyzing, previewLoading: w.previewLoading, cleaning: w.cleaning, requiresConfirmation: w.requiresConfirmation, confirmText: w.confirmText });
  const canClean = computeCanClean(w);
  const progressPath = computeProgressPath(w);

  function removeDescendantId(args: { next: Set<string>; node: AnalyzerNodeDto; id: string }) {
    const existing = w.nodesById.get(args.id);
    if (!existing) return;
    if (isAncestorAnalyzerPath(args.node.path, existing.path)) args.next.delete(args.id);
  }

  function hasAncestorSelected(args: { next: Set<string>; node: AnalyzerNodeDto }): boolean {
    return Array.from(args.next).some((id) => {
      const existing = w.nodesById.get(id);
      if (!existing) return false;
      return isAncestorAnalyzerPath(existing.path, args.node.path);
    });
  }

  function addVisibleNode(args: { next: Set<string>; id: string }) {
    const node = w.nodesById.get(args.id);
    if (!node) return;
    Array.from(args.next).forEach((id) => removeDescendantId({ next: args.next, node, id }));
    if (hasAncestorSelected({ next: args.next, node })) return;
    args.next.add(args.id);
  }

  function selectAllVisibleIds(prev: Set<string>) {
    const next = new Set(prev);
    visibleSelectableIds.forEach((id) => addVisibleNode({ next, id }));
    return next;
  }

  function deselectAllVisibleIds(prev: Set<string>) {
    const next = new Set(prev);
    visibleSelectableIds.forEach((id) => next.delete(id));
    return next;
  }

  function toggleSelectAllVisible() {
    w.invalidatePreviewRequest();
    w.setPreview(null);
    w.setCleanSummary(null);
    w.setCleanProgress(null);
    w.setSelectedIds((prev: Set<string>) => {
      if (allVisibleSelectableSelected) return deselectAllVisibleIds(prev);
      return selectAllVisibleIds(prev);
    });
  }

  function clearSelection() {
    w.invalidatePreviewRequest();
    w.setSelectedIds(new Set());
    w.setPreview(null);
  }

  return {
    allocSegments, displayPath, visibleExpandableIds, allVisibleExpanded, visibleSelectableIds, allVisibleSelectableSelected,
    visibleSelectedCount, hasResult, hasSelectedRisky, cleanProgressLabel, inspectedNode, canPreview, canClean, progressPath,
    toggleSelectAllVisible, clearSelection,
  };
}
