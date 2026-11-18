import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import type { ExecuteCleanResponse } from '../../api/client';
import type { ExplainState, PreviewResult } from '../cleanShared';
import type { PreviewRiskCalloutSource } from '../PreviewRiskCallouts';
import { ScanInspectorPanel } from './ScanInspectorPanel';
import { ScanTargetCard } from './ScanTargetCard';
import { isSelectableScanItem } from './scanHelpers';

/** Contextual end-of-results hint shown when a Scan returns only a few Targets (spec #58 R2). */
export const SCAN_RESULTS_HINT_TEXT = "That's everything found — review your selection";

/** A result set with no more than this many visible Targets is considered sparse. */
const SPARSE_RESULT_LIMIT = 8;

function shouldShowCompletionHint(visibleTargetCount: number): boolean {
  return visibleTargetCount > 0 && visibleTargetCount <= SPARSE_RESULT_LIMIT;
}

type Focus = { kind: 'item'; itemId: string } | { kind: 'group'; targetId: string } | null;
type VisibleTarget = { target: TargetScanResultDto; items: ScanItemDto[] };

interface Props {
  targets: TargetScanResultDto[];
  visibleTargets: VisibleTarget[];
  allow: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  selectionSources: readonly PreviewRiskCalloutSource[];
  collapsedTargets: Set<string>;
  showAllGroups: Set<string>;
  focus: Focus;
  focusedGroup: TargetScanResultDto | null;
  inspected: ExplainState;
  inspectedCtx: { target: TargetScanResultDto; item: ScanItemDto } | null;
  cleaning: boolean;
  scanning: boolean;
  eligibleSafeIdsLength: number;
  allEligibleSelected: boolean;
  masterIndeterminate: boolean;
  requiresConfirmation: boolean;
  confirmText: string;
  preview: PreviewResult | null;
  cleanProgressLabel: string | null;
  cleanSummary: ExecuteCleanResponse | null;
  onToggleSelectAll: () => void;
  onFocusGroup: (targetId: string) => void;
  onToggleCollapsed: (targetId: string) => void;
  onToggleGroupSelect: (target: TargetScanResultDto) => void;
  onToggleItem: (item: ScanItemDto, target: TargetScanResultDto) => void;
  onToggleShowAll: (targetId: string) => void;
  onConfirmText: (v: string) => void;
  onOpenLog: (id: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function masterAriaChecked(args: { indeterminate: boolean; checked: boolean }): 'mixed' | 'true' | 'false' {
  if (args.indeterminate) return 'mixed';
  if (args.checked) return 'true';
  return 'false';
}

function setIndeterminate(el: HTMLInputElement | null, indeterminate: boolean) {
  if (el) el.indeterminate = indeterminate;
}

function MasterCheck(props: { checked: boolean; indeterminate: boolean; disabled: boolean; onChange: () => void; label: string }) {
  const ariaChecked = masterAriaChecked({ indeterminate: props.indeterminate, checked: props.checked });
  return (
    <label className="checkbox scan-master-check" aria-label={props.label}>
      <input type="checkbox" checked={props.checked} aria-checked={ariaChecked} ref={(el) => setIndeterminate(el, props.indeterminate)} disabled={props.disabled} onChange={props.onChange} aria-label={props.label} />
    </label>
  );
}

function masterLabel(allEligibleSelected: boolean): string {
  if (allEligibleSelected) return 'Deselect all eligible files';
  return 'Select all eligible files';
}

function ScanTableHeader(props: { allEligibleSelected: boolean; masterIndeterminate: boolean; disabled: boolean; onToggle: () => void }) {
  const label = masterLabel(props.allEligibleSelected);
  return (
    <div className="scan-table-header">
      <span className="scan-th-item"><MasterCheck checked={props.allEligibleSelected} indeterminate={props.masterIndeterminate} disabled={props.disabled} onChange={props.onToggle} label={label} /><span>ITEM</span></span>
      <span className="scan-th-items num">ITEMS</span>
      <span className="scan-th-size num">SIZE</span>
      <span className="scan-th-status">STATUS</span>
    </div>
  );
}

function SkeletonPane() {
  return <div className="pane-body flush" aria-hidden="true"><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /><div className="skeleton-row" /></div>;
}

export function ScanScanningLayout(props: { allEligibleSelected: boolean; masterIndeterminate: boolean; disabled: boolean; onToggle: () => void }) {
  return (
    <section className="master-detail scan-layout" aria-label="Scanning">
      <div className="master-pane">
        <ScanTableHeader allEligibleSelected={props.allEligibleSelected} masterIndeterminate={props.masterIndeterminate} disabled={props.disabled} onToggle={props.onToggle} />
        <SkeletonPane />
      </div>
      <div className="detail-pane selection-pane scan-inspector-pane"><div className="pane-header"><h3>Inspector</h3></div><div className="detail-scroll"><p className="muted">Scanning for reclaimable space…</p></div></div>
    </section>
  );
}

function groupActive(focus: Focus, targetId: string): boolean {
  if (!focus) return false;
  if (focus.kind !== 'group') return false;
  return focus.targetId === targetId;
}

function focusedItemId(focus: Focus): string | null {
  if (!focus) return null;
  if (focus.kind !== 'item') return null;
  return focus.itemId;
}

export function ScanResultsSection(props: Props) {
  const busy = props.scanning || props.cleaning;
  const masterDisabled = props.eligibleSafeIdsLength === 0 || busy;
  return (
    <section className="master-detail scan-layout scan-layout-with-bar" aria-label="Scan results and review">
      <div className="master-pane scan-master-pane">
        <ScanTableHeader allEligibleSelected={props.allEligibleSelected} masterIndeterminate={props.masterIndeterminate} disabled={masterDisabled} onToggle={props.onToggleSelectAll} />
        <div className="pane-body flush scan-results-body scan-results-shell">
          {props.targets.length === 0 && <div className="empty-state"><h3>No targets</h3><p>No Targets returned for this Scan.</p></div>}
          <div className="target-list scan-grid-list scan-table-scroll">
            {props.visibleTargets.map(({ target, items }) => (
              <ScanTargetCard
                key={target.targetId}
                target={target}
                visibleItems={items}
                allowDeleteUnknown={props.allow}
                minAgeHours={props.minAgeHours}
                selectedIds={props.selectedIds}
                collapsed={props.collapsedTargets.has(target.targetId)}
                showAll={props.showAllGroups.has(target.targetId)}
                focusedItemId={focusedItemId(props.focus)}
                groupActive={groupActive(props.focus, target.targetId)}
                cleaning={props.cleaning}
                onFocusGroup={() => props.onFocusGroup(target.targetId)}
                onToggleCollapsed={() => props.onToggleCollapsed(target.targetId)}
                onToggleGroupSelect={() => props.onToggleGroupSelect(target)}
                onToggleItem={(item) => props.onToggleItem(item, target)}
                onToggleShowAll={() => props.onToggleShowAll(target.targetId)}
                onOpenSettings={props.onOpenSettings}
              />
            ))}
            {shouldShowCompletionHint(props.visibleTargets.length) && (
              <div className="scan-results-completion-hint" role="status" aria-label="Scan results complete">
                <span className="scan-results-completion-hint__mark" aria-hidden="true">✓</span>
                <span className="scan-results-completion-hint__text">{SCAN_RESULTS_HINT_TEXT}</span>
              </div>
            )}
          </div>
        </div>
      </div>
      <ScanInspectorPanel focus={props.focus} focusedGroup={props.focusedGroup} inspected={props.inspected} inspectedCtx={props.inspectedCtx} allowDeleteUnknown={props.allow} minAgeHours={props.minAgeHours} selectedIds={props.selectedIds} selectionSources={props.selectionSources} requiresConfirmation={props.requiresConfirmation} confirmText={props.confirmText} onConfirmText={props.onConfirmText} cleaning={props.cleaning} preview={props.preview} cleanProgressLabel={props.cleanProgressLabel} cleanSummary={props.cleanSummary} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
    </section>
  );
}
