import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath } from '../../utils/format';
import { FileIcon } from '../Icons';
import { childStatusForScan, isSelectableScanItem, itemDisplayName, statusClassForKind, statusDisplayLabel } from './scanHelpers';

interface ItemRowProps {
  item: ScanItemDto;
  target: TargetScanResultDto;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selected: boolean;
  inspected: boolean;
  cleaning: boolean;
  onToggle: () => void;
}

function isProtected(target: TargetScanResultDto): boolean {
  return target.risk === 'Protected';
}

function inspectedClass(inspected: boolean): string {
  if (inspected) return 'inspected active';
  return '';
}

function selectedClass(selected: boolean): string {
  if (selected) return 'selected';
  return '';
}

function rowClass(args: { inspected: boolean; selected: boolean }): string {
  const parts = ['scan-row scan-item-row scan-row-child scan-child-row', inspectedClass(args.inspected), selectedClass(args.selected)];
  return parts.filter(Boolean).join(' ');
}

function statusAria(args: { kind: string; label: string }): string {
  if (args.label.startsWith('Skipped')) return 'Status Skipped';
  if (args.kind === 'skipped') return 'Status Skipped';
  return `Status ${args.label}`;
}

function Subpath(props: { nestedRel?: string; name: string }) {
  if (!props.nestedRel) return null;
  if (props.nestedRel === props.name) return null;
  return <span className="scan-subpath-inline mono" title={props.nestedRel}>{middleTruncatePath({ path: props.nestedRel, maxLen: 64 })}</span>;
}

function ProtectedSlot(props: { path: string; selected: boolean }) {
  return <><span className="fixed-check-placeholder" aria-hidden="true">—</span><input className="sr-only" type="checkbox" aria-label={`Select ${props.path}`} disabled checked={props.selected} readOnly tabIndex={-1} /></>;
}

function DisabledSlot(props: { path: string; selected: boolean }) {
  return <><span className="scan-disabled-checkbox-outline" aria-hidden="true" /><input className="sr-only" type="checkbox" aria-label={`Select ${props.path}`} disabled checked={props.selected} readOnly tabIndex={-1} /></>;
}

function SelectSlot(props: { path: string; selected: boolean; cleaning: boolean; onToggle: () => void }) {
  return <label className="checkbox"><input type="checkbox" aria-label={`Select ${props.path}`} checked={props.selected} disabled={props.cleaning} onChange={props.onToggle} /></label>;
}

function ItemSelect(props: { path: string; selectable: boolean; protectedTarget: boolean; selected: boolean; cleaning: boolean; onToggle: () => void }) {
  if (props.protectedTarget) return <ProtectedSlot path={props.path} selected={props.selected} />;
  if (!props.selectable) return <DisabledSlot path={props.path} selected={props.selected} />;
  return <SelectSlot path={props.path} selected={props.selected} cleaning={props.cleaning} onToggle={props.onToggle} />;
}

function onRowKeyDown(e: React.KeyboardEvent<HTMLDivElement>, onToggle: () => void) {
  if (e.target !== e.currentTarget) return;
  if (e.key === 'Enter') { e.preventDefault(); onToggle(); return; }
  if (e.key === ' ') { e.preventDefault(); onToggle(); }
}

export function ScanItemRow(props: ItemRowProps) {
  const selectable = isSelectableScanItem({ item: props.item, target: props.target, allowDeleteUnknown: props.allowDeleteUnknown });
  const { name, nestedRel } = itemDisplayName({ itemPath: props.item.path, resolvedPaths: props.target.resolvedPaths });
  const status = childStatusForScan({ target: props.target, item: props.item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  const fullPath = normalizeDisplayPath({ p: props.item.path });
  const protectedTarget = isProtected(props.target);
  return (
    <div className={rowClass({ inspected: props.inspected, selected: props.selected })} role="button" tabIndex={0} onClick={props.onToggle} onKeyDown={(e) => onRowKeyDown(e, props.onToggle)}>
      <div className="scan-col-main">
        <span className="row-indent" aria-hidden="true" />
        <span className="scan-select-slot" onClick={(e) => e.stopPropagation()}><ItemSelect path={fullPath} selectable={selectable} protectedTarget={protectedTarget} selected={props.selected} cleaning={props.cleaning} onToggle={props.onToggle} /></span>
        <FileIcon className="row-icon" aria-hidden="true" />
        <div className="scan-main-text"><div className="scan-title-line"><span className="item-name" title={fullPath}>{middleTruncatePath({ path: name, maxLen: 64 })}</span><Subpath nestedRel={nestedRel} name={name} /></div></div>
      </div>
      <div className="scan-col-items muted" />
      <div className="scan-col-size num">{formatBytes({ bytes: props.item.size })}</div>
      <div className="scan-col-status"><span className={statusClassForKind({ kind: status.kind, label: status.label })} aria-label={statusAria({ kind: status.kind, label: status.label })}>{statusDisplayLabel({ kind: status.kind, label: status.label })}</span></div>
    </div>
  );
}