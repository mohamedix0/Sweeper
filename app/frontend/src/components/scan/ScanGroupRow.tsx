import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath } from '../../utils/format';
import { ChevronDownIcon, ChevronRightIcon, FolderIcon } from '../Icons';
import { customDisplayName, groupItemStatus, isCustomPathTargetId, isSelectableScanItem, resolvedRootPath, statusClassForKind, statusDisplayLabel } from './scanHelpers';

interface GroupRowProps {
  target: TargetScanResultDto;
  allowDeleteUnknown: boolean;
  selectedIds: Set<string>;
  collapsed: boolean;
  groupActive: boolean;
  cleaning: boolean;
  onFocusGroup: () => void;
  onToggleCollapsed: () => void;
  onToggleGroupSelect: () => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function selectableItems(target: TargetScanResultDto, allowDeleteUnknown: boolean) {
  return target.items.filter((i) => isSelectableScanItem({ item: i, target, allowDeleteUnknown }));
}

function onRowKeyDown(e: React.KeyboardEvent<HTMLDivElement>, action: () => void) {
  if (e.target !== e.currentTarget) return;
  if (e.key === 'Enter') { e.preventDefault(); action(); return; }
  if (e.key === ' ') { e.preventDefault(); action(); }
}

function collapseLabel(args: { collapsed: boolean; name: string }): string {
  if (args.collapsed) return `Expand ${args.name}`;
  return `Collapse ${args.name}`;
}

function collapseIcon(collapsed: boolean) {
  if (collapsed) return <ChevronRightIcon />;
  return <ChevronDownIcon />;
}

function CollapseButton(props: { collapsed: boolean; name: string; onClick: () => void }) {
  return <button type="button" className="target-collapse-btn" onClick={(e) => { e.stopPropagation(); props.onClick(); }} aria-label={collapseLabel({ collapsed: props.collapsed, name: props.name })}>{collapseIcon(props.collapsed)}</button>;
}

function isFullySelected(args: { total: number; selected: number }): boolean {
  if (args.total <= 0) return false;
  return args.selected === args.total;
}

function isIndeterminate(args: { selected: number; total: number }): boolean {
  if (args.selected <= 0) return false;
  return args.selected < args.total;
}

function ariaChecked(args: { indeterminate: boolean; full: boolean }): boolean | 'mixed' {
  if (args.indeterminate) return 'mixed';
  return args.full;
}

function ProtectedGroupCheckbox(props: { name: string }) {
  return <><span className="fixed-check-placeholder" aria-hidden="true">—</span><input className="sr-only" type="checkbox" aria-label={`Select all eligible items in ${props.name}`} disabled checked={false} readOnly tabIndex={-1} /></>;
}

function GroupCheckbox(props: { target: TargetScanResultDto; selectableIds: string[]; selectedInTarget: number; cleaning: boolean; onChange: () => void }) {
  if (props.target.risk === 'Protected') return <ProtectedGroupCheckbox name={props.target.targetName} />;
  const total = props.selectableIds.length;
  const full = isFullySelected({ total, selected: props.selectedInTarget });
  const indeterminate = isIndeterminate({ selected: props.selectedInTarget, total });
  const disabled = props.cleaning || props.selectableIds.length === 0;
  return <label className="checkbox target-group-check"><input type="checkbox" aria-label={`Select all eligible items in ${props.target.targetName}`} checked={full} aria-checked={ariaChecked({ indeterminate, full })} ref={(el) => { if (el) el.indeterminate = indeterminate; }} disabled={disabled} onChange={props.onChange} /></label>;
}

function UnknownSr(props: { targetName: string }) {
  if (props.targetName !== 'Custom Path (Unknown)') return null;
  return <span className="sr-only">Custom Path (Unknown)</span>;
}

function TitleExtras(props: { display: ReturnType<typeof customDisplayName>; targetName: string }) {
  if (!props.display.tag) return null;
  return <><span className="scan-custom-path-tag muted">{props.display.tag}</span><UnknownSr targetName={props.targetName} /></>;
}

function SizeSecondary(props: { show: boolean; total: string }) {
  if (!props.show) return null;
  return <div className="scan-group-size-secondary muted">{`of ${formatBytes({ bytes: props.total })}`}</div>;
}

function StatusBadge(props: { status: ReturnType<typeof groupItemStatus>; target: TargetScanResultDto; locked: boolean; onOpenSettings?: GroupRowProps['onOpenSettings'] }) {
  if (!props.status) return <span className="scan-status-empty muted">—</span>;
  const cls = statusClassForKind({ kind: props.status.kind, label: props.status.label });
  const text = statusDisplayLabel({ kind: props.status.kind, label: props.status.label });
  if (props.locked && props.onOpenSettings) {
    return <button type="button" className={`${cls} scan-status-action-pill scan-status-action-pill-interactive`} aria-label={props.status.aria} title="Click to configure in Settings" onClick={(e) => { e.stopPropagation(); props.onOpenSettings?.('allowDeleteUnknown', resolvedRootPath({ target: props.target })); }}>{text} →</button>;
  }
  return <span className={cls} aria-label={props.status.aria}>{text}</span>;
}

function isCustomUnknownLocked(target: TargetScanResultDto, allowDeleteUnknown: boolean): boolean {
  if (!isCustomPathTargetId({ targetId: target.targetId })) return false;
  if (target.targetName !== 'Custom Path (Unknown)') return false;
  if (target.risk !== 'Unknown') return false;
  return !allowDeleteUnknown;
}

function rowClass(active: boolean): string {
  if (active) return 'scan-row scan-group-row active-group';
  return 'scan-row scan-group-row';
}

function eligibleForStatusBytes(args: { locked: boolean; bytes: bigint }): bigint {
  if (args.locked) return 0n;
  return args.bytes;
}

function rootPathText(rootPath: string): string {
  const text = middleTruncatePath({ path: rootPath, maxLen: 64 });
  if (text) return text;
  return '\u00A0';
}

export function ScanGroupRow(props: GroupRowProps) {
  const selectable = selectableItems(props.target, props.allowDeleteUnknown);
  const selectableIds = selectable.map((i) => i.id);
  const selectedInTarget = selectableIds.filter((id) => props.selectedIds.has(id)).length;
  const display = customDisplayName({ target: props.target });
  const rootPath = resolvedRootPath({ target: props.target });
  const totalScannedBytes = BigInt(props.target.totalBytes || '0');
  const eligibleBytesBig = selectable.reduce((acc, it) => acc + BigInt(it.size || '0'), 0n);
  const locked = isCustomUnknownLocked(props.target, props.allowDeleteUnknown);
  const groupStatus = groupItemStatus({ target: props.target, items: props.target.items, selectableItems: selectable, selectedInTarget, isCustomUnknownLocked: locked });
  const eligibleForStatus = eligibleForStatusBytes({ locked, bytes: eligibleBytesBig });
  return (
    <div className={rowClass(props.groupActive)} role="button" tabIndex={0} aria-label={`Inspect ${props.target.targetName}`} onClick={props.onFocusGroup} onKeyDown={(e) => onRowKeyDown(e, props.onFocusGroup)}>
      <div className="scan-col-main">
        <CollapseButton collapsed={props.collapsed} name={props.target.targetName} onClick={props.onToggleCollapsed} />
        <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}><GroupCheckbox target={props.target} selectableIds={selectableIds} selectedInTarget={selectedInTarget} cleaning={props.cleaning} onChange={props.onToggleGroupSelect} /></span>
        <FolderIcon className="row-icon" aria-hidden="true" />
        <div className="scan-main-text scan-group-main-text">
          <div className="scan-title-line scan-title-line-with-action"><span className="target-name-btn scan-group-title" id={`target-${props.target.targetId}-name`}>{display.title}</span><TitleExtras display={display} targetName={props.target.targetName} /></div>
          <div className="scan-subpath scan-group-subpath" title={rootPath}><span className="mono scan-group-path-text">{rootPathText(rootPath)}</span></div>
        </div>
      </div>
      <div className="scan-col-items muted num">{`${selectedInTarget} of ${props.target.items.length} selected`}</div>
      <div className="scan-col-size scan-group-size-col num"><div className="scan-group-size-primary">{formatBytes({ bytes: eligibleForStatus.toString() })}</div><SizeSecondary show={totalScannedBytes > eligibleForStatus} total={props.target.totalBytes} /></div>
      <div className="scan-col-status scan-group-status-col"><StatusBadge status={groupStatus} target={props.target} locked={locked} onOpenSettings={props.onOpenSettings} /></div>
    </div>
  );
}