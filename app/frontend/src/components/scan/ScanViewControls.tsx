import type { ScanToolbarState } from './scanHelpers';

interface Props {
  toolbar: ScanToolbarState;
  targetsCount: number;
  selectableCount: number;
  skippedCount: number;
  hasResults: boolean;
  cleaning: boolean;
  anyCollapsed: boolean;
  isFilterActive: boolean;
  selectSafeIdsLength: number;
  allSelectSafeSelected: boolean;
  onToolbar: (next: ScanToolbarState) => void;
  onToggleCollapseAll: () => void;
  onSelectAllSafe: () => void;
}

function baseSelectSafeLabel(args: { isFilterActive: boolean; count: number }): string {
  if (args.isFilterActive) return `Select Visible Safe (${args.count})`;
  return 'Select Safe';
}

function selectSafeLabel(args: { isFilterActive: boolean; count: number; selected: boolean }): string {
  const base = baseSelectSafeLabel(args);
  if (args.selected) return `✓ ${base}`;
  return base;
}

function filterButtonClass(active: boolean): string {
  if (active) return 'seg-btn active';
  return 'seg-btn';
}

function FilterButton(props: { active: boolean; label: string; ariaLabel?: string; pressed: boolean; onClick: () => void }) {
  return <button type="button" className={filterButtonClass(props.active)} aria-pressed={props.pressed} aria-label={props.ariaLabel} onClick={props.onClick}>{props.label}</button>;
}

function collapseLabel(args: { anyCollapsed: boolean }): string {
  if (args.anyCollapsed) return 'Expand All';
  return 'Collapse All';
}

function CollapseButton(props: { cleaning: boolean; hasResults: boolean; anyCollapsed: boolean; onClick: () => void }) {
  const disabled = props.cleaning || !props.hasResults;
  return <button className="btn small subtle-btn" onClick={props.onClick} disabled={disabled}>{collapseLabel({ anyCollapsed: props.anyCollapsed })}</button>;
}

function selectSafeAria(args: { isFilterActive: boolean; count: number }): string {
  if (args.isFilterActive) return `Select Visible Safe (${args.count})`;
  return 'Select Safe';
}

function selectSafeTitle(args: { selected: boolean }): string | undefined {
  if (args.selected) return 'All safe items selected';
  return undefined;
}

function SelectSafeButton(props: { cleaning: boolean; count: number; isFilterActive: boolean; selected: boolean; label: string; onClick: () => void }) {
  const disabled = props.cleaning || props.count === 0;
  const title = selectSafeTitle({ selected: props.selected });
  return <button className="btn small" onClick={props.onClick} disabled={disabled} aria-label={selectSafeAria({ isFilterActive: props.isFilterActive, count: props.count })} title={title}>{props.label}</button>;
}

export function ScanViewControls(props: Props) {
  const label = selectSafeLabel({ isFilterActive: props.isFilterActive, count: props.selectSafeIdsLength, selected: props.allSelectSafeSelected });
  return (
    <section className="scan-controls scan-controls-compact" aria-label="Scan controls">
      <div className="scan-table-toolbar scan-table-toolbar-mainbar" aria-label="Scan table toolbar">
        <div className="scan-table-toolbar-left">
          <div className="segmented-toggle scan-status-segmented" role="group" aria-label="Status filters">
            <FilterButton active={props.toolbar.filter === 'all'} label={`All (${props.targetsCount} categories · ${props.selectableCount + props.skippedCount} files)`} pressed={props.toolbar.filter === 'all'} onClick={() => props.onToolbar({ ...props.toolbar, filter: 'all' })} />
            <FilterButton active={props.toolbar.filter === 'eligible'} label={`Eligible (${props.selectableCount})`} ariaLabel={`Eligible (${props.selectableCount} files)`} pressed={props.toolbar.filter === 'eligible'} onClick={() => props.onToolbar({ ...props.toolbar, filter: 'eligible' })} />
            <FilterButton active={props.toolbar.filter === 'skipped'} label={`Skipped (${props.skippedCount})`} pressed={props.toolbar.filter === 'skipped'} onClick={() => props.onToolbar({ ...props.toolbar, filter: 'skipped' })} />
          </div>
        </div>
        <div className="scan-table-toolbar-middle">
          <input type="search" className="scan-files-search" placeholder="Search files..." value={props.toolbar.query} onChange={(e) => props.onToolbar({ ...props.toolbar, query: e.target.value })} aria-label="Search files" disabled={!props.hasResults} />
        </div>
        <div className="scan-table-toolbar-right">
          <label className="toolbar-group scan-sort-group">Sort<select value={props.toolbar.sort} onChange={(e) => props.onToolbar({ ...props.toolbar, sort: e.target.value as 'size' | 'name' })} aria-label="Sort scan results" disabled={!props.hasResults}><option value="size">Size</option><option value="name">Name</option></select></label>
          <CollapseButton cleaning={props.cleaning} hasResults={props.hasResults} anyCollapsed={props.anyCollapsed} onClick={props.onToggleCollapseAll} />
          <SelectSafeButton cleaning={props.cleaning} count={props.selectSafeIdsLength} isFilterActive={props.isFilterActive} selected={props.allSelectSafeSelected} label={label} onClick={props.onSelectAllSafe} />
        </div>
      </div>
    </section>
  );
}
