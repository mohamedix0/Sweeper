import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath } from '../../utils/format';
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, FileIcon, FolderIcon } from '../Icons';
import { childStatusForScan, customDisplayName, groupItemStatus, isCustomPathTargetId, isSelectableScanItem, itemDisplayName, resolvedRootPath, statusClassForKind, statusDisplayLabel, VISIBLE_CHILD_CAP } from './scanHelpers';

function ItemRow(props: { item: ScanItemDto; target: TargetScanResultDto; allowDeleteUnknown: boolean; minAgeHours?: number | null; selected: boolean; inspected: boolean; cleaning: boolean; onToggle: () => void }) {
  const selectable = isSelectableScanItem({ item: props.item, target: props.target, allowDeleteUnknown: props.allowDeleteUnknown });
  const { name, nestedRel } = itemDisplayName({ itemPath: props.item.path, resolvedPaths: props.target.resolvedPaths });
  const isProtected = props.target.risk === 'Protected';
  const status = childStatusForScan({ target: props.target, item: props.item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  const isStatusSkipped = status.label.startsWith('Skipped') || status.kind === 'skipped';
  return (
    <div className={`scan-row scan-item-row scan-row-child scan-child-row ${props.inspected ? 'inspected active' : ''} ${props.selected ? 'selected' : ''}`} role="button" tabIndex={0} onClick={props.onToggle} onKeyDown={(e) => { if (e.target !== e.currentTarget) return; if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); props.onToggle(); } }}>
      <div className="scan-col-main">
        <span className="row-indent" aria-hidden="true" />
        <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
          {isProtected ? (<><span className="fixed-check-placeholder" aria-hidden="true">—</span><input className="sr-only" type="checkbox" aria-label={`Select ${normalizeDisplayPath(props.item.path)}`} disabled checked={props.selected} readOnly tabIndex={-1} /></>) : !selectable ? (<><span className="scan-disabled-checkbox-outline" aria-hidden="true" /><input className="sr-only" type="checkbox" aria-label={`Select ${normalizeDisplayPath(props.item.path)}`} disabled checked={props.selected} readOnly tabIndex={-1} /></>) : (<label className="checkbox"><input type="checkbox" aria-label={`Select ${normalizeDisplayPath(props.item.path)}`} checked={props.selected} disabled={props.cleaning} onChange={props.onToggle} /></label>)}
        </span>
        <FileIcon className="row-icon" aria-hidden="true" />
        <div className="scan-main-text"><div className="scan-title-line"><span className="item-name" title={normalizeDisplayPath(props.item.path)}>{middleTruncatePath(name, 64)}</span>{nestedRel && nestedRel !== name ? <span className="scan-subpath-inline mono" title={nestedRel}>{middleTruncatePath(nestedRel, 64)}</span> : null}</div></div>
      </div>
      <div className="scan-col-items muted" />
      <div className="scan-col-size num">{formatBytes(props.item.size)}</div>
      <div className="scan-col-status"><span className={statusClassForKind(status.kind, status.label)} aria-label={isStatusSkipped ? 'Status Skipped' : `Status ${status.label}`}>{statusDisplayLabel(status.kind, status.label)}</span></div>
    </div>
  );
}

export function ScanTargetCard(props: {
  target: TargetScanResultDto;
  visibleItems: ScanItemDto[];
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  collapsed: boolean;
  showAll: boolean;
  focusedItemId: string | null;
  groupActive: boolean;
  cleaning: boolean;
  onFocusGroup: () => void;
  onToggleCollapsed: () => void;
  onToggleGroupSelect: () => void;
  onToggleItem: (item: ScanItemDto) => void;
  onToggleShowAll: () => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}) {
  const t = props.target;
  const selectableItems = t.items.filter((i) => isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: props.allowDeleteUnknown }));
  const selectableIds = selectableItems.map((i) => i.id);
  const selectedInTarget = selectableIds.filter((id) => props.selectedIds.has(id)).length;
  const allEligibleInGroupSelected = selectableIds.length > 0 && selectedInTarget === selectableIds.length;
  const isFullySelected = t.items.length > 0 && selectedInTarget === t.items.length;
  const isIndeterminate = selectedInTarget > 0 && selectedInTarget < t.items.length;
  const limitedItems = !props.showAll && props.visibleItems.length > VISIBLE_CHILD_CAP ? props.visibleItems.slice(0, VISIBLE_CHILD_CAP) : props.visibleItems;
  const hiddenCount = !props.showAll ? Math.max(0, props.visibleItems.length - limitedItems.length) : 0;
  const isCustomUnknownLocked = isCustomPathTargetId(t.targetId) && t.targetName === 'Custom Path (Unknown)' && t.risk === 'Unknown' && !props.allowDeleteUnknown;
  const display = customDisplayName(t);
  const rootPath = resolvedRootPath(t);
  const rootPathDisplay = middleTruncatePath(rootPath, 64);
  const totalScannedBytes = BigInt(t.totalBytes || '0');
  const eligibleBytesBig = isCustomUnknownLocked ? 0n : selectableItems.reduce((acc, it) => acc + BigInt(it.size || '0'), 0n);
  const groupStatus = groupItemStatus({ target: t, items: t.items, selectableItems, selectedInTarget, isCustomUnknownLocked });
  return (
    <article className={`target-card ${props.collapsed ? 'collapsed' : 'expanded'}`} aria-labelledby={`target-${t.targetId}-name`}>
      <div className={`scan-row scan-group-row${props.groupActive ? ' active-group' : ''}`} role="button" tabIndex={0} aria-label={`Inspect ${t.targetName}`} onClick={props.onFocusGroup} onKeyDown={(e) => { if (e.target !== e.currentTarget) return; if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); props.onFocusGroup(); } }}>
        <div className="scan-col-main">
          <button type="button" className="target-collapse-btn" onClick={(e) => { e.stopPropagation(); props.onToggleCollapsed(); }} aria-label={props.collapsed ? `Expand ${t.targetName}` : `Collapse ${t.targetName}`}>{props.collapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}</button>
          <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}>
            {t.risk === 'Protected' ? (<><span className="fixed-check-placeholder" aria-hidden="true">—</span><input className="sr-only" type="checkbox" aria-label={`Select all eligible items in ${t.targetName}`} disabled checked={false} readOnly tabIndex={-1} /></>) : (<label className="checkbox target-group-check"><input type="checkbox" aria-label={`Select all eligible items in ${t.targetName}`} checked={isFullySelected} aria-checked={isIndeterminate ? 'mixed' : isFullySelected} ref={(el) => { if (el) el.indeterminate = isIndeterminate; }} disabled={props.cleaning || selectableIds.length === 0} onChange={props.onToggleGroupSelect} /></label>)}
          </span>
          <FolderIcon className="row-icon" aria-hidden="true" />
          <div className="scan-main-text scan-group-main-text">
            <div className="scan-title-line scan-title-line-with-action"><span className="target-name-btn scan-group-title" id={`target-${t.targetId}-name`}>{display.title}</span>{display.tag && <span className="scan-custom-path-tag muted">{display.tag}</span>}{t.targetName === 'Custom Path (Unknown)' && <span className="sr-only">Custom Path (Unknown)</span>}</div>
            <div className="scan-subpath scan-group-subpath" title={rootPath}><span className="mono scan-group-path-text">{rootPathDisplay || '\u00A0'}</span></div>
          </div>
        </div>
        <div className="scan-col-items muted num">{`${selectedInTarget} of ${t.items.length} selected`}</div>
        <div className="scan-col-size scan-group-size-col num"><div className="scan-group-size-primary">{formatBytes(eligibleBytesBig.toString())}</div>{totalScannedBytes > eligibleBytesBig && <div className="scan-group-size-secondary muted">{`of ${formatBytes(t.totalBytes)}`}</div>}</div>
        <div className="scan-col-status scan-group-status-col">
          {groupStatus ? (isCustomUnknownLocked && props.onOpenSettings ? (<button type="button" className={`${statusClassForKind(groupStatus.kind, groupStatus.label)} scan-status-action-pill scan-status-action-pill-interactive`} aria-label={groupStatus.aria} title="Click to configure in Settings" onClick={(e) => { e.stopPropagation(); props.onOpenSettings?.('allowDeleteUnknown', resolvedRootPath(t)); }}>{statusDisplayLabel(groupStatus.kind, groupStatus.label)} →</button>) : (<span className={statusClassForKind(groupStatus.kind, groupStatus.label)} aria-label={groupStatus.aria}>{statusDisplayLabel(groupStatus.kind, groupStatus.label)}</span>)) : <span className="scan-status-empty muted">—</span>}
        </div>
      </div>
      {!props.collapsed && (
        <div className="items" role="group" aria-label={`${t.targetName} items`}>
          <div className="scan-items-guide" aria-hidden="true" />
          {t.items.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items.</span></div>}
          {t.items.length > 0 && props.visibleItems.length === 0 && <div className="scan-row scan-item-empty"><span className="muted">No items shown for current filters.</span></div>}
          {limitedItems.map((item) => <ItemRow key={item.id} item={item} target={t} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} selected={props.selectedIds.has(item.id)} inspected={props.focusedItemId === item.id} cleaning={props.cleaning} onToggle={() => props.onToggleItem(item)} />)}
          {hiddenCount > 0 && <div className="scan-row scan-item-empty scan-show-all-row"><button type="button" className="btn small subtle-btn" onClick={props.onToggleShowAll}>{`Showing top ${VISIBLE_CHILD_CAP} of ${props.visibleItems.length} files · Show more`}</button></div>}
          {props.showAll && props.visibleItems.length > VISIBLE_CHILD_CAP && <div className="scan-row scan-item-empty scan-show-all-row"><button type="button" className="btn small subtle-btn" onClick={props.onToggleShowAll}>Show fewer files</button></div>}
        </div>
      )}
    </article>
  );
}
