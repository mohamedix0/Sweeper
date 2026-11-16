import type { TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, normalizeDisplayPath, pluralize } from '../../utils/format';
import { InspectorTitleRow } from '../cleanShared';
import { FolderIcon } from '../Icons';
import { childStatusForScan, customDisplayName, isCustomPathTargetId, isSelectableScanItem, itemDisplayName, resolvedRootPath, statusClassForKind, statusDisplayLabel, sumDecimalStrings } from './scanHelpers';

interface GroupProps {
  target: TargetScanResultDto;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function selectableForTarget(props: GroupProps) {
  return props.target.items.filter((i) => isSelectableScanItem({ item: i, target: props.target, allowDeleteUnknown: props.allowDeleteUnknown }));
}

function isCustomUnknownLocked(target: TargetScanResultDto, allowDeleteUnknown: boolean): boolean {
  if (!isCustomPathTargetId({ targetId: target.targetId })) return false;
  if (target.targetName !== 'Custom Path (Unknown)') return false;
  if (target.risk !== 'Unknown') return false;
  return !allowDeleteUnknown;
}

function groupStatusKind(args: { locked: boolean; risk: TargetScanResultDto['risk'] }): string {
  if (args.locked) return 'skipped';
  if (args.risk === 'Protected') return 'protected';
  if (args.risk === 'Caution') return 'caution';
  if (args.risk === 'Unknown') return 'unknown';
  return 'safe';
}

function groupStatusLabel(args: { locked: boolean; risk: TargetScanResultDto['risk'] }): string {
  if (args.locked) return 'Skipped · Unverified';
  return args.risk;
}

function agePolicyLabel(minAgeHours?: number | null): string {
  if (minAgeHours === 0) return 'Policy: Minimum age (none)';
  const hours = minAgeHours ?? 'configured';
  return `Policy: Minimum age > ${hours}h`;
}

function policySuffix(targetName: string): string {
  if (targetName === 'User Temp') return 'Auto-recreated by Windows/apps';
  return `Governed by ${targetName} safety rules`;
}

function policyText(args: { targetName: string; minAgeHours?: number | null }): string {
  return `${agePolicyLabel(args.minAgeHours)} · ${policySuffix(args.targetName)}`;
}

function fallbackDescription(target: TargetScanResultDto): string {
  return target.statusReason || 'Category-level cleanup target governed by Sweeper safety rules.';
}

function descriptionText(target: TargetScanResultDto): string {
  if (target.items.length === 0) return fallbackDescription(target);
  if (target.targetName === 'User Temp') return 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.';
  return fallbackDescription(target);
}

function compareSizesDesc(a: { size: string }, b: { size: string }): number {
  const sa = BigInt(a.size);
  const sb = BigInt(b.size);
  if (sa === sb) return 0;
  if (sa > sb) return -1;
  return 1;
}

function topFiles(target: TargetScanResultDto) {
  return [...target.items].sort(compareSizesDesc).slice(0, 5);
}

function fileStatus(args: { target: TargetScanResultDto; item: TargetScanResultDto['items'][number]; allowDeleteUnknown: boolean; minAgeHours?: number | null }) {
  const selectable = isSelectableScanItem({ item: args.item, target: args.target, allowDeleteUnknown: args.allowDeleteUnknown });
  return childStatusForScan({ target: args.target, item: args.item, selectable, allowDeleteUnknown: args.allowDeleteUnknown, minAgeHours: args.minAgeHours });
}

function TopFileRow(props: { target: TargetScanResultDto; item: TargetScanResultDto['items'][number]; allowDeleteUnknown: boolean; minAgeHours?: number | null }) {
  const name = itemDisplayName({ itemPath: props.item.path, resolvedPaths: props.target.resolvedPaths }).name;
  const st = fileStatus({ target: props.target, item: props.item, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  return <li><span className="inspector-top-file-name" title={normalizeDisplayPath({ p: props.item.path })}>{name}</span><span className="num muted">{formatBytes({ bytes: props.item.size })}</span><span className="muted">{statusDisplayLabel({ kind: st.kind, label: st.label })}</span></li>;
}

function TopFilesList(props: GroupProps) {
  const items = topFiles(props.target);
  if (items.length === 0) return <p className="muted small-note">No files in this category.</p>;
  return <ul className="inspector-top-files-list">{items.map((item) => <TopFileRow key={item.id} target={props.target} item={item} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} />)}</ul>;
}

function AllowSettingsBlock(props: { target: TargetScanResultDto; onOpenSettings: NonNullable<GroupProps['onOpenSettings']> }) {
  return (
    <div className="inspector-policy-links inspector-allow-settings-prominent" aria-label="Inspector actions">
      <div className="inspector-rule-safety-block"><div className="inspector-rule-safety-title">ELIGIBILITY / RULE &amp; SAFETY</div><p className="muted small-note">Unverified custom folder is locked by Settings. Enable custom path cleanup to make it eligible.</p></div>
      <button type="button" className="btn small secondary-action inspector-inline-link prominent-allow-link" onClick={() => props.onOpenSettings('allowDeleteUnknown', resolvedRootPath({ target: props.target }))}>Allow in Settings →</button>
    </div>
  );
}

function AllowBlock(props: { show: boolean; target: TargetScanResultDto; onOpenSettings?: GroupProps['onOpenSettings'] }) {
  if (!props.show) return null;
  if (!props.onOpenSettings) return null;
  return <AllowSettingsBlock target={props.target} onOpenSettings={props.onOpenSettings} />;
}

function bytesText(value: string): string {
  if (value) return value;
  return '0';
}

function skippedItems(props: GroupProps) {
  return props.target.items.filter((i) => !isSelectableScanItem({ item: i, target: props.target, allowDeleteUnknown: props.allowDeleteUnknown }));
}

function GroupBadge(props: { kind: string; label: string }) {
  return <span className={statusClassForKind({ kind: props.kind as never, label: props.label })}>{props.label}</span>;
}

export function ScanGroupInspector(props: GroupProps) {
  const selectableItems = selectableForTarget(props);
  const selectedInTarget = selectableItems.filter((i) => props.selectedIds.has(i.id)).length;
  const skippedInTarget = props.target.items.length - selectableItems.length;
  const locked = isCustomUnknownLocked(props.target, props.allowDeleteUnknown);
  const kind = groupStatusKind({ locked, risk: props.target.risk });
  const label = groupStatusLabel({ locked, risk: props.target.risk });
  const display = customDisplayName({ target: props.target });
  const eligibleBytes = bytesText(sumDecimalStrings({ values: selectableItems.map((i) => i.size) }).toString());
  const skippedBytes = bytesText(sumDecimalStrings({ values: skippedItems(props).map((i) => i.size) }).toString());
  return (
    <>
      <InspectorTitleRow icon={<FolderIcon className="inspector-leading-icon" />} title={display.title} subtitle="Category Overview" badge={<GroupBadge kind={kind} label={label} />} />
      <p className="inspector-guidance">{descriptionText(props.target)}</p>
      <dl className="inspector-meta-grid inspector-meta-grid-v19" aria-label="Category metadata">
        <div><dt>Reclaimable</dt><dd className="num">{formatBytes({ bytes: eligibleBytes })}</dd></div>
        <div><dt>Skipped</dt><dd className="num">{formatBytes({ bytes: skippedBytes })}</dd></div>
        <div className="inspector-meta-eligibility inspector-meta-eligibility-full"><dt>Items</dt><dd>{selectedInTarget} selected · {skippedInTarget} skipped · {pluralize({ count: props.target.items.length, singular: 'file', plural: 'files' })} total</dd></div>
      </dl>
      <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety"><div className="inspector-policy-title">Rule &amp; Safety</div><div className="inspector-policy-text"><strong>{policyText({ targetName: props.target.targetName, minAgeHours: props.minAgeHours })}</strong></div></div>
      <div className="inspector-top-files" aria-label="Top files preview"><strong className="subheading-xs">Top files</strong><TopFilesList {...props} /></div>
      <AllowBlock show={locked} target={props.target} onOpenSettings={props.onOpenSettings} />
    </>
  );
}