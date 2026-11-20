import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { ExecuteCleanResponse, Settings, StartAnalyzerResult, SweeperClient } from '../../api/client';
import { AnalyzerCleanPanel } from './AnalyzerCleanPanel';
import { AnalyzerInspectorPanel } from './AnalyzerInspectorPanel';
import { AnalyzerTreeRow } from './AnalyzerTreeRow';
import { AnalyzerToolbar } from './AnalyzerViewParts';
import type { AnalyzerStatusFilter, FlatAnalyzerRow } from './analyzerHelpers';
import type { ExplainState, PreviewResult } from '../cleanShared';
import type { PreviewRiskCalloutSource } from '../PreviewRiskCallouts';
import { activeDescendantFor, consumePendingShowAllFocus, handleTreeKeyDown, resolveActiveId } from './analyzerTreeKeyboard';
import { activationCallbacks } from './analyzerTreeActivation';
import { ANALYZER_TREE_DOM_ID, treeItemDomId } from './analyzerTreeA11y';

type ResultsProps = {
  rows: FlatAnalyzerRow[];
  rootTotalBytes: bigint;
  expanded: Set<string>;
  selectedIds: Set<string>;
  inspectedId: string | null;
  allowDeleteUnknown: boolean;
  cleaning: boolean;
  analyzing: boolean;
  previewLoading: boolean;
  statusFilter: AnalyzerStatusFilter;
  statusCounts: { all: number; safe: number; caution: number; unknown: number; locked: number };
  query: string;
  sort: 'size' | 'items' | 'name';
  allVisibleExpanded: boolean;
  visibleSelectableCount: number;
  visibleSelectedCount: number;
  allVisibleSelectableSelected: boolean;
  inspected: ExplainState;
  inspectedNode: AnalyzerNodeDto | null;
  selectionSources: readonly PreviewRiskCalloutSource[];
  requiresConfirmation: boolean;
  confirmText: string;
  preview: PreviewResult | null;
  cleanProgressLabel: string | null;
  cleanSummary: ExecuteCleanResponse | null;
  selectedCount: number;
  selectedBytes: string;
  hasSelectedRisky: boolean;
  canPreview: boolean;
  canClean: boolean;
  onStatus: (v: AnalyzerStatusFilter) => void;
  onQuery: (v: string) => void;
  onSort: (v: 'size' | 'items' | 'name') => void;
  onSelectSafe: () => void;
  onToggleExpandCollapse: () => void;
  onToggleSelectAll: () => void;
  onToggleExpand: (node: AnalyzerNodeDto) => void;
  onToggleSelect: (node: AnalyzerNodeDto) => void;
  onInspect: (args: { id: string }) => void;
  onShowMore: (args: { parentId: string }) => void;
  onDrill: (args: { path: string }) => void;
  onConfirmText: (v: string) => void;
  onOpenLog: (id: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
  onClear: () => void;
  onPreview: () => void;
  onClean: () => void;
};

function masterCheckboxDisabled(args: { analyzing: boolean; cleaning: boolean; count: number }): boolean {
  if (args.analyzing) return true;
  if (args.cleaning) return true;
  return args.count === 0;
}

function setIndeterminate(el: HTMLInputElement | null, indeterminate: boolean) {
  if (!el) return;
  el.indeterminate = indeterminate;
}

function focusDomElement(id: string | null) {
  if (!id) return;
  document.getElementById(id)?.focus();
}

function masterCheckboxLabel(allVisibleSelectableSelected: boolean): string {
  return allVisibleSelectableSelected ? 'Deselect all visible selectable nodes' : 'Select all visible selectable nodes';
}

function masterCheckboxIndeterminate(visibleSelectedCount: number, visibleSelectableCount: number): boolean {
  return visibleSelectedCount > 0 && visibleSelectedCount < visibleSelectableCount;
}

function emptyTreeState(rowCount: number) {
  return rowCount === 0 ? <div className="muted analyzer-empty">No nodes.</div> : null;
}

export function AnalyzerResults(props: ResultsProps) {
  const [activeId, setActiveId] = useState<string | null>(null);
  const pendingShowAllFocusRef = useRef<string | null>(null);
  const revealTargetRef = useRef<string | null>(null);
  const revealOnReconcileRef = useRef<string | null>(null);
  const suppressReconcileRevealRef = useRef(false);
  const rowIds = useMemo(() => props.rows.map((row) => row.node.id), [props.rows]);

  useEffect(() => {
    const next = resolveActiveId(rowIds, activeId);
    const suppressReveal = suppressReconcileRevealRef.current;
    suppressReconcileRevealRef.current = false;
    if (next !== activeId) {
      // A query / analysis change can remove the active row. Reveal the chosen
      // fallback so a sighted keyboard user is not left with an off-screen
      // active row. A Show more / Show fewer replacement must NOT reveal: the
      // focused replacement control has to stay in view.
      revealOnReconcileRef.current = suppressReveal ? null : next;
      setActiveId(next);
    }
  }, [rowIds, activeId]);

  useLayoutEffect(() => {
    focusDomElement(consumePendingShowAllFocus(pendingShowAllFocusRef));
  }, [props.rows]);

  // Reveal the keyboard-active row: ArrowUp/ArrowDown call preventDefault, so
  // without this the viewport never scrolls and focus can land on invisible
  // rows. Two causes reveal: arrow navigation (recorded by onArrowNavigation)
  // and a query / analysis change that removes the active row (recorded by the
  // reconciliation effect above). A Show more / Show fewer replacement is
  // explicitly excluded so the focused replacement control stays in view.
  // jsdom has no scrollIntoView, hence the function guard.
  useEffect(() => {
    if (!activeId) return;
    if (revealTargetRef.current !== activeId && revealOnReconcileRef.current !== activeId) return;
    revealTargetRef.current = null;
    revealOnReconcileRef.current = null;
    const element = document.getElementById(treeItemDomId(activeId));
    if (typeof element?.scrollIntoView === 'function') element.scrollIntoView({ block: 'nearest' });
  }, [activeId]);

  function handleShowMore(args: { parentId: string }) {
    // A Show more / Show fewer replacement removes the activated pseudo-row;
    // suppress the reconciliation reveal so the focused control stays in view.
    suppressReconcileRevealRef.current = true;
    pendingShowAllFocusRef.current = args.parentId;
    props.onShowMore(args);
  }

  function onArrowNavigation(id: string) {
    if (id !== activeId) revealTargetRef.current = id;
  }

  function onTreeKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const activations = activationCallbacks({
      rows: props.rows,
      allowDeleteUnknown: props.allowDeleteUnknown,
      cleaning: props.cleaning,
      onInspect: props.onInspect,
      onToggleSelect: props.onToggleSelect,
      onToggleExpand: props.onToggleExpand,
      onShowMore: handleShowMore,
    });
    handleTreeKeyDown(e, { rowIds, activeId, setActiveId, onArrowNavigation, ...activations });
  }

  const activeDescendant = activeDescendantFor(rowIds, activeId);
  const indeterminate = masterCheckboxIndeterminate(props.visibleSelectedCount, props.visibleSelectableCount);
  const checkboxLabel = masterCheckboxLabel(props.allVisibleSelectableSelected);
  return (
    <>
      <section className="master-detail analyzer-layout scan-layout-with-bar" aria-label="Analyzer results">
        <div className="master-pane">
          <h2 id="analyzer-tree-heading" className="sr-only">Analyzer Tree</h2>
          <AnalyzerToolbar statusFilter={props.statusFilter} statusCounts={props.statusCounts} query={props.query} sort={props.sort} hasResult analyzing={props.analyzing} cleaning={props.cleaning} allVisibleExpanded={props.allVisibleExpanded} onStatus={props.onStatus} onQuery={props.onQuery} onSort={props.onSort} onSelectSafe={props.onSelectSafe} onToggleExpandCollapse={props.onToggleExpandCollapse} />
          <div className="analyzer-col-headers">
            <span className="analyzer-col-h-name">
              <span className="analyzer-header-master-slot">
                <label className="checkbox analyzer-master-check" aria-label={checkboxLabel}>
                  <input type="checkbox" aria-label={checkboxLabel} checked={props.allVisibleSelectableSelected} ref={(el) => setIndeterminate(el, indeterminate)} disabled={masterCheckboxDisabled({ analyzing: props.analyzing, cleaning: props.cleaning, count: props.visibleSelectableCount })} onChange={props.onToggleSelectAll} />
                </label>
              </span>
              <span aria-hidden="true">Name</span>
            </span>
            <span className="analyzer-col-h-share" aria-hidden="true">Share</span><span className="analyzer-col-h-items" aria-hidden="true">Files</span><span className="analyzer-col-h-size" aria-hidden="true">Size</span><span className="analyzer-col-h-status" aria-hidden="true">Status</span>
          </div>
          <div className="pane-body flush">
            <div id={ANALYZER_TREE_DOM_ID} className="analyzer-viewport" role="tree" aria-label="Analyzer results tree" aria-activedescendant={activeDescendant} tabIndex={0} onKeyDown={onTreeKeyDown}>
              {props.rows.map((row) => <AnalyzerTreeRow key={row.node.id} row={row} rootTotalBytes={props.rootTotalBytes} expanded={props.expanded} selectedIds={props.selectedIds} inspectedId={props.inspectedId} active={row.node.id === activeId} allowDeleteUnknown={props.allowDeleteUnknown} cleaning={props.cleaning} onToggleExpand={props.onToggleExpand} onToggleSelect={props.onToggleSelect} onInspect={props.onInspect} onShowMore={handleShowMore} onDrill={props.onDrill} />)}
              {emptyTreeState(props.rows.length)}
            </div>
          </div>
        </div>
        <AnalyzerInspectorPanel inspected={props.inspected} inspectedNode={props.inspectedNode} selectedIds={props.selectedIds} selectionSources={props.selectionSources} allowDeleteUnknown={props.allowDeleteUnknown} requiresConfirmation={props.requiresConfirmation} confirmText={props.confirmText} onConfirmText={props.onConfirmText} preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} cleanSummary={props.cleanSummary} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      </section>
      <AnalyzerCleanPanel selectedCount={props.selectedCount} selectedBytes={props.selectedBytes} hasPreview={Boolean(props.preview)} hasSelectedRisky={props.hasSelectedRisky} cleaning={props.cleaning} previewLoading={props.previewLoading} canPreview={props.canPreview} canClean={props.canClean} onClear={props.onClear} onPreview={props.onPreview} onClean={props.onClean} />
    </>
  );
}
