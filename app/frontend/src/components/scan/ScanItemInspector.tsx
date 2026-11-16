import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { normalizeDisplayPath } from '../../utils/format';
import { InspectorMetaGrid, InspectorTitleRow, PathActions, PathWell, formatModifiedAge, type ExplainState } from '../cleanShared';
import { FileIcon } from '../Icons';
import { childStatusForScan, customDisplayName, isCustomPathTargetId, isSelectableScanItem, relativeItemName, resolvedRootPath, scanGuidance, statusClassForKind, statusDisplayLabel, humanizeSkippedReason } from './scanHelpers';

interface ItemProps {
  target: TargetScanResultDto;
  item: ScanItemDto;
  inspected: ExplainState;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function itemTitle(args: { item: ScanItemDto; target: TargetScanResultDto; fallback: string }): string {
  const rel = relativeItemName({ itemPath: args.item.path, resolvedPaths: args.target.resolvedPaths });
  const name = rel.split(/[\\/]/).pop();
  if (name) return name;
  return args.fallback;
}

function eligibleAgeText(args: { minAgeHours?: number | null }): string {
  if (args.minAgeHours == null) return 'Meets age limit';
  if (args.minAgeHours === 0) return 'Eligible (no minimum age)';
  return `Meets age limit (> ${args.minAgeHours}h old)`;
}

function eligibilityText(args: { selectable: boolean; minAgeHours?: number | null; target: TargetScanResultDto; item: ScanItemDto; allowDeleteUnknown: boolean }): string {
  if (args.selectable) return eligibleAgeText({ minAgeHours: args.minAgeHours });
  return humanizeSkippedReason({ target: args.target, item: args.item, allowDeleteUnknown: args.allowDeleteUnknown, minAgeHours: args.minAgeHours });
}

function agePolicyValue(args: { minAgeHours?: number | null }): string {
  if (args.minAgeHours === 0) return '(none)';
  if (args.minAgeHours == null) return '> configured';
  return `> ${args.minAgeHours}h`;
}

function policyForTarget(args: { targetName: string; minAgeHours?: number | null }): string {
  const prefix = `Minimum age ${agePolicyValue({ minAgeHours: args.minAgeHours })}`;
  if (args.targetName === 'User Temp') return `${prefix} · Auto-recreated by Windows/apps`;
  return `${prefix} · Governed by ${args.targetName} safety rules`;
}

function VhdxNote() {
  return <div className="notice info" role="note"><div><strong>.vhdx is display-only.</strong> To compact: shut down WSL/VM (<code>wsl --shutdown</code>) then run <code>Optimize-VHD</code> (Hyper-V) or your VM tooling.</div></div>;
}

function VhdxBlock(props: { path: string }) {
  if (!props.path.toLowerCase().endsWith('.vhdx')) return null;
  return <VhdxNote />;
}

function customUnknownLocked(args: { target: TargetScanResultDto; allowDeleteUnknown: boolean }): boolean {
  if (!isCustomPathTargetId({ targetId: args.target.targetId })) return false;
  if (args.target.risk !== 'Unknown') return false;
  return !args.allowDeleteUnknown;
}

function showAgeAction(args: { selectable: boolean; ageEligible: boolean }): boolean {
  if (args.selectable) return false;
  return !args.ageEligible;
}

function AllowUnknownButton(props: { show: boolean; target: TargetScanResultDto; onOpenSettings?: ItemProps['onOpenSettings'] }) {
  if (!props.show) return null;
  if (!props.onOpenSettings) return null;
  return <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.('allowDeleteUnknown', resolvedRootPath({ target: props.target }))}>Allow in Settings →</button>;
}

function AgeButton(props: { show: boolean; onOpenSettings?: ItemProps['onOpenSettings'] }) {
  if (!props.show) return null;
  if (!props.onOpenSettings) return null;
  return <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.()}>Adjust minimum age in Settings →</button>;
}

function ItemActions(props: { show: boolean; target: TargetScanResultDto; locked: boolean; ageAction: boolean; onOpenSettings?: ItemProps['onOpenSettings'] }) {
  if (!props.show) return null;
  return (
    <div className="inspector-policy-links" aria-label="Inspector actions">
      <AllowUnknownButton show={props.locked} target={props.target} onOpenSettings={props.onOpenSettings} />
      <AgeButton show={props.ageAction} onOpenSettings={props.onOpenSettings} />
    </div>
  );
}

function statusAria(args: { kind: string; label: string }): string {
  if (args.label.startsWith('Skipped')) return 'Status Skipped';
  if (args.kind === 'skipped') return 'Status Skipped';
  return `Status ${args.label}`;
}

function subtitleText(args: { display: ReturnType<typeof customDisplayName>; targetName: string }): string {
  if (args.display.tag) return args.display.title;
  return args.targetName;
}

export function ScanItemInspector(props: ItemProps) {
  const { target, item } = props;
  const selectable = isSelectableScanItem({ item, target, allowDeleteUnknown: props.allowDeleteUnknown });
  const fullPath = normalizeDisplayPath({ p: item.path });
  const title = itemTitle({ item, target, fallback: props.inspected?.title || '' });
  const status = childStatusForScan({ target, item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  const eligibility = eligibilityText({ selectable, minAgeHours: props.minAgeHours, target, item, allowDeleteUnknown: props.allowDeleteUnknown });
  const modifiedAge = formatModifiedAge({ mtimeUnixSecs: item.mtimeUnixSecs, ageEligible: item.ageEligible, minAgeHours: props.minAgeHours ?? undefined });
  const display = customDisplayName({ target });
  const policy = policyForTarget({ targetName: target.targetName, minAgeHours: props.minAgeHours });
  const locked = customUnknownLocked({ target, allowDeleteUnknown: props.allowDeleteUnknown });
  const ageAction = showAgeAction({ selectable, ageEligible: item.ageEligible });
  const hasSettings = !!props.onOpenSettings;
  const needsActions = locked || ageAction;
  const showActions = hasSettings && needsActions;
  return (
    <>
      <InspectorTitleRow icon={<FileIcon className="inspector-leading-icon" />} title={title} subtitle={subtitleText({ display, targetName: target.targetName })} badge={<span className={statusClassForKind({ kind: status.kind, label: status.label })} aria-label={statusAria({ kind: status.kind, label: status.label })}>{statusDisplayLabel({ kind: status.kind, label: status.label })}</span>} />
      <p className="inspector-guidance">{scanGuidance({ target, item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours })}</p>
      <InspectorMetaGrid size={item.size} modifiedAge={modifiedAge} eligibility={eligibility} />
      <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety"><div className="inspector-policy-title">Rule &amp; Safety</div><div className="inspector-policy-text"><strong>Policy:</strong> {policy}</div></div>
      <VhdxBlock path={fullPath} />
      <PathWell path={fullPath} /><PathActions path={fullPath} />
      <ItemActions show={showActions} target={target} locked={locked} ageAction={ageAction} onOpenSettings={props.onOpenSettings} />
    </>
  );
}