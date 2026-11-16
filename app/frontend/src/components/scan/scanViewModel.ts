import type { TargetScanResultDto } from '../../contract/bindings';
import { isSelectableScanItem, scanSelectionNeedsConfirmation } from './scanHelpers';

type TargetsArgs = { targets: TargetScanResultDto[]; allow: boolean };
type SelArgs = TargetsArgs & { selectedIds: Set<string> };

export function countSelectable(args: TargetsArgs): number {
  return args.targets.reduce((n, t) => n + t.items.filter((i) => isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: args.allow })).length, 0);
}

export function countSkipped(args: TargetsArgs): number {
  return args.targets.reduce((n, t) => n + t.items.filter((i) => !isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: args.allow })).length, 0);
}

function collectSelectableIds(args: { items: TargetScanResultDto['items']; target: TargetScanResultDto; allow: boolean }): string[] {
  return args.items.filter((i) => isSelectableScanItem({ item: i, target: args.target, allowDeleteUnknown: args.allow })).map((i) => i.id);
}

function safeTargetIds(args: { target: TargetScanResultDto; items: TargetScanResultDto['items']; allow: boolean }): string[] {
  if (args.target.risk !== 'Safe') return [];
  return collectSelectableIds({ items: args.items, target: args.target, allow: args.allow });
}

export function eligibleSafeIdsFor(args: TargetsArgs): string[] {
  return args.targets.flatMap((t) => safeTargetIds({ target: t, items: t.items, allow: args.allow }));
}

export function visibleEligibleSafeIdsFor(args: { visibleTargets: Array<{ target: TargetScanResultDto; items: TargetScanResultDto['items'] }>; allow: boolean }): string[] {
  return args.visibleTargets.flatMap(({ target, items }) => safeTargetIds({ target, items, allow: args.allow }));
}

function targetHasSelected(args: { target: TargetScanResultDto; selectedIds: Set<string> }): boolean {
  return args.target.items.some((i) => args.selectedIds.has(i.id));
}

export function selectedCategoryCount(args: SelArgs): number {
  return args.targets.filter((t) => targetHasSelected({ target: t, selectedIds: args.selectedIds })).length;
}

export function needsConfirmation(args: SelArgs): boolean {
  return scanSelectionNeedsConfirmation({ targets: args.targets, selectedIds: args.selectedIds });
}

export function heroReclaimableBytes(args: { targets: TargetScanResultDto[] }): string {
  return args.targets.reduce((a, t) => a + BigInt(t.eligibleBytes || '0'), 0n).toString();
}
