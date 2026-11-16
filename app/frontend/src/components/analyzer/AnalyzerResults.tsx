import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { ExecuteCleanResponse, Settings, StartAnalyzerResult, SweeperClient } from '../../api/client';
import { AnalyzerCleanPanel } from './AnalyzerCleanPanel';
import { AnalyzerInspectorPanel } from './AnalyzerInspectorPanel';
import { AnalyzerTreeRow } from './AnalyzerTreeRow';
import { AnalyzerToolbar } from './AnalyzerViewParts';
import type { AnalyzerStatusFilter, FlatAnalyzerRow } from './analyzerHelpers';
import type { ExplainState, PreviewResult } from '../cleanShared';

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

export function AnalyzerResults(props: ResultsProps) {
  const indeterminate = props.visibleSelectedCount > 0 && props.visibleSelectedCount < props.visibleSelectableCount;
  function checkboxLabel(): string {
    if (props.allVisibleSelectableSelected) return 'Deselect all visible selectable nodes';
    return 'Select all visible selectable nodes';
  }
  return (
    <>
      <section className="master-detail analyzer-layout scan-layout-with-bar" aria-label="Analyzer results">
        <div className="master-pane">
          <h2 id="analyzer-tree-heading" className="sr-only">Analyzer Tree</h2>
          <AnalyzerToolbar statusFilter={props.statusFilter} statusCounts={props.statusCounts} query={props.query} sort={props.sort} hasResult analyzing={props.analyzing} cleaning={props.cleaning} allVisibleExpanded={props.allVisibleExpanded} onStatus={props.onStatus} onQuery={props.onQuery} onSort={props.onSort} onSelectSafe={props.onSelectSafe} onToggleExpandCollapse={props.onToggleExpandCollapse} />
          <div className="analyzer-col-headers">
            <span className="analyzer-col-h-name">
              <span className="analyzer-header-master-slot">
                <label className="checkbox analyzer-master-check" aria-label={checkboxLabel()}>
                  <input type="checkbox" aria-label={checkboxLabel()} checked={props.allVisibleSelectableSelected} ref={(el) => setIndeterminate(el, indeterminate)} disabled={masterCheckboxDisabled({ analyzing: props.analyzing, cleaning: props.cleaning, count: props.visibleSelectableCount })} onChange={props.onToggleSelectAll} />
                </label>
              </span>
              <span aria-hidden="true">Name</span>
            </span>
            <span className="analyzer-col-h-share" aria-hidden="true">Share</span><span className="analyzer-col-h-items" aria-hidden="true">Files</span><span className="analyzer-col-h-size" aria-hidden="true">Size</span><span className="analyzer-col-h-status" aria-hidden="true">Status</span>
          </div>
          <div className="pane-body flush">
            <div className="analyzer-viewport" role="tree" aria-label="Analyzer results tree" tabIndex={0}>
              {props.rows.map((row) => <AnalyzerTreeRow key={row.node.id} row={row} rootTotalBytes={props.rootTotalBytes} expanded={props.expanded} selectedIds={props.selectedIds} inspectedId={props.inspectedId} allowDeleteUnknown={props.allowDeleteUnknown} cleaning={props.cleaning} onToggleExpand={props.onToggleExpand} onToggleSelect={props.onToggleSelect} onInspect={props.onInspect} onShowMore={props.onShowMore} onDrill={props.onDrill} />)}
              {props.rows.length === 0 && <div className="muted analyzer-empty">No nodes.</div>}
            </div>
          </div>
        </div>
        <AnalyzerInspectorPanel inspected={props.inspected} inspectedNode={props.inspectedNode} allowDeleteUnknown={props.allowDeleteUnknown} requiresConfirmation={props.requiresConfirmation} confirmText={props.confirmText} onConfirmText={props.onConfirmText} preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} cleanSummary={props.cleanSummary} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      </section>
      <AnalyzerCleanPanel selectedCount={props.selectedCount} selectedBytes={props.selectedBytes} hasPreview={Boolean(props.preview)} hasSelectedRisky={props.hasSelectedRisky} cleaning={props.cleaning} previewLoading={props.previewLoading} canPreview={props.canPreview} canClean={props.canClean} onClear={props.onClear} onPreview={props.onPreview} onClean={props.onClean} />
    </>
  );
}
