import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath, sumDecimalStrings } from '../../utils/format';

export type ScanFilter = 'all' | 'eligible' | 'skipped';
export type ScanSort = 'size' | 'name';
export type ScanToolbarState = { query: string; sort: ScanSort; filter: ScanFilter };
export type FocusState = { kind: 'item'; itemId: string } | { kind: 'group'; targetId: string } | null;
export type ScanStatusKind = 'safe' | 'skipped' | 'caution' | 'unknown' | 'protected';
export type ScanStatus = { label: string; kind: ScanStatusKind };
export const VISIBLE_CHILD_CAP = 25;

type ItemCtx = { item: ScanItemDto; target: TargetScanResultDto; allowDeleteUnknown: boolean; minAgeHours?: number | null };
type GroupStatusArgs = { target: TargetScanResultDto; items: ScanItemDto[]; selectableItems: ScanItemDto[]; selectedInTarget: number; isCustomUnknownLocked: boolean };

export function isCustomPathTargetId(args: { targetId: string }): boolean {
  return args.targetId === 'custom-path' || args.targetId.startsWith('custom-path');
}

export function isSelectableScanItem(args: { item: { ageEligible: boolean }; target: TargetScanResultDto; allowDeleteUnknown: boolean }): boolean {
  if (args.target.risk === 'Protected') return false;
  if (!args.item.ageEligible) return false;
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return false;
  return true;
}

function selectedItemNeedsConfirmation(args: { target: TargetScanResultDto; itemId: string; selectedIds: Set<string> }): boolean {
  if (!args.selectedIds.has(args.itemId)) return false;
  if (args.target.risk === 'Unknown') return true;
  return isCustomPathTargetId({ targetId: args.target.targetId });
}

function targetNeedsConfirmation(args: { target: TargetScanResultDto; selectedIds: Set<string> }): boolean {
  return args.target.items.some((item) => selectedItemNeedsConfirmation({ target: args.target, itemId: item.id, selectedIds: args.selectedIds }));
}

export function scanSelectionNeedsConfirmation(args: { targets: TargetScanResultDto[]; selectedIds: Set<string> }): boolean {
  return args.targets.some((target) => targetNeedsConfirmation({ target, selectedIds: args.selectedIds }));
}

export function formatMinAgeLabel(args: { minAgeHours: number | null | undefined }): string {
  if (args.minAgeHours == null) return 'minimum age';
  if (args.minAgeHours === 0) return 'no minimum age';
  return `${args.minAgeHours}h`;
}

function skippedRecentLabel(args: { minAgeHours?: number | null }): string {
  const label = formatMinAgeLabel({ minAgeHours: args.minAgeHours });
  if (label === 'minimum age') return 'Skipped · Recent';
  if (label === 'no minimum age') return 'Skipped · Recent';
  return `Skipped · < ${label}`;
}

export function humanizeSkippedReason(args: ItemCtx): string {
  if (args.target.risk === 'Protected') return 'Protected';
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return 'Skipped · Unverified';
  if (!args.item.ageEligible) return skippedRecentLabel({ minAgeHours: args.minAgeHours });
  return 'Skipped · Not eligible';
}

function normPathForCompare(p: string): string {
  return p.replace(/[\\/]+$/, '').toLowerCase();
}

function relativeFromResolved(args: { itemPath: string; rp: string }): string | null {
  const base = normPathForCompare(args.rp);
  const low = args.itemPath.toLowerCase();
  if (low.startsWith(base + '\\')) return args.itemPath.slice(args.rp.replace(/[\\/]+$/, '').length + 1) || args.itemPath;
  if (low.startsWith(base + '/')) return args.itemPath.slice(args.rp.replace(/[\\/]+$/, '').length + 1) || args.itemPath;
  return null;
}

export function relativeItemName(args: { itemPath: string; resolvedPaths: string[] }): string {
  const rel = args.resolvedPaths.map((rp) => relativeFromResolved({ itemPath: args.itemPath, rp })).find((v): v is string => v != null);
  if (rel) return rel;
  const parts = args.itemPath.split(/[\\/]/);
  return parts[parts.length - 1] || args.itemPath;
}

export function itemDisplayName(args: { itemPath: string; resolvedPaths: string[] }): { name: string; nestedRel: string } {
  const rel = relativeItemName(args);
  const parts = rel.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return { name: parts[0] || rel, nestedRel: '' };
  return { name: parts[parts.length - 1] || rel, nestedRel: rel };
}

export function resolvedRootPath(args: { target: TargetScanResultDto }): string {
  return normalizeDisplayPath({ p: args.target.resolvedPaths[0] || '' });
}

function isUnverifiedLocked(args: ItemCtx): boolean {
  if (isCustomPathTargetId({ targetId: args.target.targetId }) && args.target.risk === 'Unknown') return true;
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return true;
  return false;
}

function skippedStatusFor(args: ItemCtx): ScanStatus {
  if (isUnverifiedLocked(args)) return { label: 'Skipped · Unverified', kind: 'skipped' };
  if (!args.item.ageEligible) return { label: skippedRecentLabel({ minAgeHours: args.minAgeHours }), kind: 'skipped' };
  return { label: humanizeSkippedReason(args), kind: 'skipped' };
}

function selectableRiskStatus(args: ItemCtx): ScanStatus {
  if (args.target.risk === 'Caution') return { label: 'Caution', kind: 'caution' };
  if (args.target.risk === 'Unknown') return { label: 'Unknown', kind: 'unknown' };
  return { label: 'Safe', kind: 'safe' };
}

export function childStatusForScan(args: ItemCtx & { selectable: boolean }): ScanStatus {
  if (args.target.risk === 'Protected') return { label: 'Protected', kind: 'protected' };
  if (!args.selectable) return skippedStatusFor(args);
  return selectableRiskStatus(args);
}

const STATUS_KIND_CLASS: Record<string, string> = {
  eligible: 'status-pill status-pill-safe status-pill-scan',
  safe: 'status-pill status-pill-safe status-pill-scan',
  skipped: 'status-pill status-pill-caution status-pill-scan',
  caution: 'status-pill status-pill-caution status-pill-scan',
  unknown: 'status-pill status-pill-unknown status-pill-scan badge-unknown',
  protected: 'status-pill status-pill-protected status-pill-scan',
};

export function statusClassForKind(args: { kind: ScanStatusKind | 'eligible'; label?: string }): string {
  if (args.label === 'Skipped · Unverified') return 'status-pill status-pill-unknown status-pill-scan badge-unknown';
  return STATUS_KIND_CLASS[args.kind] ?? 'status-pill status-pill-scan';
}

export function statusDisplayLabel(args: { kind: ScanStatusKind | 'eligible'; label: string }): string {
  if (args.kind === 'eligible') return 'Eligible';
  if (args.kind === 'safe') return 'Safe';
  return args.label;
}

function isCustomUnknown(args: ItemCtx): boolean {
  if (!isCustomPathTargetId({ targetId: args.target.targetId })) return false;
  return args.target.risk === 'Unknown';
}

function lockedUnknownGuidance(args: ItemCtx): string | null {
  if (args.target.risk !== 'Unknown') return null;
  if (isCustomUnknown(args) && !args.allowDeleteUnknown) return 'Unverified custom folder is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible, then review carefully.';
  if (!args.allowDeleteUnknown) return 'Unknown item is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible.';
  if (isCustomUnknown(args)) return 'Unverified custom folder item. Review carefully; typed DELETE confirmation is required before cleaning.';
  return 'Unknown item. Review carefully; typed DELETE confirmation is required before cleaning.';
}

function guidanceLockedTarget(args: ItemCtx): string | null {
  if (args.target.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  return lockedUnknownGuidance(args);
}

function guidanceForRecent(args: { minAgeHours?: number | null }): string {
  if (args.minAgeHours === 0) return 'Recently modified. Sweeper skips recent files for safety under the current minimum age setting.';
  if (args.minAgeHours == null) return 'Modified within the minimum age window. Sweeper skips recent files for safety.';
  return `Modified within the minimum age window (${args.minAgeHours}h). Sweeper skips recent files for safety.`;
}

function guidanceByTargetName(args: ItemCtx): string | null {
  if (args.target.targetName === 'User Temp') return 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.';
  if (args.target.targetName === 'Crash Dumps & WER') return 'Local crash and error reports kept for diagnostics. Removing them does not affect app functionality.';
  if (args.target.targetName.toLowerCase().includes('npm')) return 'Package manager download cache. It can be redownloaded automatically when needed.';
  if (isCustomPathTargetId({ targetId: args.target.targetId })) return 'Custom folder item. Only selected contents are cleaned and protected system directories remain excluded.';
  return null;
}

export function scanGuidance(args: ItemCtx & { selectable: boolean }): string {
  const locked = guidanceLockedTarget(args);
  if (locked) return locked;
  if (!args.item.ageEligible) return guidanceForRecent({ minAgeHours: args.minAgeHours });
  const byName = guidanceByTargetName(args);
  if (byName) return byName;
  if (args.target.risk === 'Caution') return 'Caution category. Review before removal; selected items are permanently deleted.';
  if (args.target.risk === 'Safe') return 'Safe to remove when not in active use. Selected items are permanently deleted after review and confirmation.';
  return 'Eligible under current safety settings. Selected items are permanently deleted after review and confirmation.';
}

export function customDisplayName(args: { target: TargetScanResultDto }): { title: string; tag: string | null } {
  if (args.target.targetName !== 'Custom Path (Unknown)') return { title: args.target.targetName, tag: null };
  const base = (resolvedRootPath({ target: args.target }).split('\\').filter(Boolean).pop()) || args.target.targetName;
  return { title: base, tag: 'Custom Path' };
}

function isEmptyProtectedTarget(target: TargetScanResultDto): boolean {
  if (target.risk !== 'Protected') return false;
  return BigInt(target.totalBytes || '0') === 0n;
}

export function visibleBaseTargets(args: { targets: TargetScanResultDto[] }): TargetScanResultDto[] {
  return args.targets.filter((t) => !isEmptyProtectedTarget(t));
}

function groupRiskKind(args: GroupStatusArgs): ScanStatusKind {
  if (args.target.risk === 'Caution') return 'caution';
  if (args.target.risk === 'Unknown') return 'unknown';
  return 'safe';
}

function groupRiskLabel(args: GroupStatusArgs): string {
  if (args.target.risk === 'Caution') return 'Caution';
  if (args.target.risk === 'Unknown') return 'Unknown';
  return 'Safe';
}

function groupPartialLabel(args: { eligible: number; riskLabel: string }): string {
  if (args.eligible === 1) return `1 ${args.riskLabel}`;
  return `Partial ${args.riskLabel}`;
}

function fixedGroupStatus(args: GroupStatusArgs): { label: string; kind: 'skipped' | 'protected'; aria: string } | null {
  if (args.isCustomUnknownLocked) return { label: 'Skipped · Unverified', kind: 'skipped', aria: 'Status Skipped' };
  if (args.target.risk === 'Protected') return { label: 'Protected', kind: 'protected', aria: 'Status Protected' };
  const total = args.items.length;
  const eligible = args.selectableItems.length;
  if (eligible === 0 && total > 0) return { label: 'Skipped', kind: 'skipped', aria: 'Status Skipped' };
  return null;
}

export function groupItemStatus(args: GroupStatusArgs) {
  const fixed = fixedGroupStatus(args);
  if (fixed) return fixed;
  const eligible = args.selectableItems.length;
  if (eligible === 0) return null;
  const kind = groupRiskKind(args);
  const label = groupRiskLabel(args);
  const skipped = args.items.length - eligible;
  if (skipped === 0) return { label, kind, aria: `Status ${label}` };
  return { label: groupPartialLabel({ eligible, riskLabel: label }), kind, aria: `Status ${label}` };
}

function compareScanSize(a: ScanItemDto, b: ScanItemDto): number {
  const da = BigInt(a.size);
  const db = BigInt(b.size);
  if (da === db) return 0;
  if (da > db) return -1;
  return 1;
}

export function sortScanItems(args: { items: ScanItemDto[]; target: TargetScanResultDto; sort: ScanSort }): ScanItemDto[] {
  const next = [...args.items];
  if (args.sort === 'name') {
    return next.sort((a, b) => relativeItemName({ itemPath: a.path, resolvedPaths: args.target.resolvedPaths }).localeCompare(relativeItemName({ itemPath: b.path, resolvedPaths: args.target.resolvedPaths })));
  }
  return next.sort(compareScanSize);
}

function matchesFilter(args: { filter: ScanFilter; selectable: boolean }): boolean {
  if (args.filter === 'eligible' && !args.selectable) return false;
  if (args.filter === 'skipped' && args.selectable) return false;
  return true;
}

function matchesQuery(args: { item: ScanItemDto; target: TargetScanResultDto; q: string }): boolean {
  if (!args.q) return true;
  const rel = relativeItemName({ itemPath: args.item.path, resolvedPaths: args.target.resolvedPaths }).toLowerCase();
  const name = rel.split(/[\\/]/).pop()?.toLowerCase() ?? '';
  if (args.item.path.toLowerCase().includes(args.q)) return true;
  if (rel.includes(args.q)) return true;
  return name.includes(args.q);
}

export function filterScanItems(args: { items: ScanItemDto[]; target: TargetScanResultDto; toolbar: ScanToolbarState; allowDeleteUnknown: boolean }): ScanItemDto[] {
  const q = args.toolbar.query.trim().toLowerCase();
  return args.items.filter((item) => {
    const selectable = isSelectableScanItem({ item, target: args.target, allowDeleteUnknown: args.allowDeleteUnknown });
    if (!matchesFilter({ filter: args.toolbar.filter, selectable })) return false;
    return matchesQuery({ item, target: args.target, q });
  });
}

export { formatBytes, middleTruncatePath, normalizeDisplayPath, sumDecimalStrings };
