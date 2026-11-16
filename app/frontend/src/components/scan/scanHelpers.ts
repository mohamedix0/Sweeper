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

export function isCustomPathTargetId(targetId: string): boolean {
  return targetId === 'custom-path' || targetId.startsWith('custom-path');
}

export function isSelectableScanItem(args: { item: { ageEligible: boolean }; target: TargetScanResultDto; allowDeleteUnknown: boolean }): boolean {
  if (args.target.risk === 'Protected') return false;
  if (!args.item.ageEligible) return false;
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return false;
  return true;
}

export function scanSelectionNeedsConfirmation(args: { targets: TargetScanResultDto[]; selectedIds: Set<string> }): boolean {
  for (const t of args.targets) {
    for (const item of t.items) {
      if (!args.selectedIds.has(item.id)) continue;
      if (t.risk === 'Unknown' || isCustomPathTargetId(t.targetId)) return true;
    }
  }
  return false;
}

export function formatMinAgeLabel(minAgeHours: number | null | undefined): string {
  if (minAgeHours == null) return 'minimum age';
  if (minAgeHours === 0) return 'no minimum age';
  return `${minAgeHours}h`;
}

function skippedRecentLabel(minAgeHours?: number | null): string {
  const label = formatMinAgeLabel(minAgeHours);
  if (label === 'minimum age' || label === 'no minimum age') return 'Skipped · Recent';
  return `Skipped · < ${label}`;
}

export function humanizeSkippedReason(args: ItemCtx): string {
  if (args.target.risk === 'Protected') return 'Protected';
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return 'Skipped · Unverified';
  if (!args.item.ageEligible) return skippedRecentLabel(args.minAgeHours);
  return 'Skipped · Not eligible';
}

export function relativeItemName(args: { itemPath: string; resolvedPaths: string[] }): string {
  const norm = (p: string) => p.replace(/[\\/]+$/, '').toLowerCase();
  const low = args.itemPath.toLowerCase();
  for (const rp of args.resolvedPaths) {
    const base = norm(rp);
    if (low.startsWith(base + '\\') || low.startsWith(base + '/')) return args.itemPath.slice(rp.replace(/[\\/]+$/, '').length + 1) || args.itemPath;
  }
  const parts = args.itemPath.split(/[\\/]/);
  return parts[parts.length - 1] || args.itemPath;
}

export function itemDisplayName(args: { itemPath: string; resolvedPaths: string[] }): { name: string; nestedRel: string } {
  const rel = relativeItemName(args);
  const parts = rel.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return { name: parts[0] || rel, nestedRel: '' };
  return { name: parts[parts.length - 1] || rel, nestedRel: rel };
}

export function resolvedRootPath(target: TargetScanResultDto): string {
  return normalizeDisplayPath(target.resolvedPaths[0] || '');
}

export function childStatusForScan(args: ItemCtx & { selectable: boolean }): ScanStatus {
  if (args.target.risk === 'Protected') return { label: 'Protected', kind: 'protected' };
  if (!args.selectable) {
    if ((isCustomPathTargetId(args.target.targetId) && args.target.risk === 'Unknown') || (args.target.risk === 'Unknown' && !args.allowDeleteUnknown)) return { label: 'Skipped · Unverified', kind: 'skipped' };
    if (!args.item.ageEligible) return { label: skippedRecentLabel(args.minAgeHours), kind: 'skipped' };
    return { label: humanizeSkippedReason(args), kind: 'skipped' };
  }
  if (args.target.risk === 'Caution') return { label: 'Caution', kind: 'caution' };
  if (args.target.risk === 'Unknown') return { label: 'Unknown', kind: 'unknown' };
  return { label: 'Safe', kind: 'safe' };
}

export function statusClassForKind(kind: ScanStatusKind | 'eligible', label?: string): string {
  if (kind === 'eligible') return 'status-pill status-pill-safe status-pill-scan';
  if (label === 'Skipped · Unverified') return 'status-pill status-pill-unknown status-pill-scan badge-unknown';
  if (kind === 'safe') return 'status-pill status-pill-safe status-pill-scan';
  if (kind === 'skipped' || kind === 'caution') return 'status-pill status-pill-caution status-pill-scan';
  if (kind === 'unknown') return 'status-pill status-pill-unknown status-pill-scan badge-unknown';
  if (kind === 'protected') return 'status-pill status-pill-protected status-pill-scan';
  return 'status-pill status-pill-scan';
}

export function statusDisplayLabel(kind: ScanStatusKind | 'eligible', label: string): string {
  if (kind === 'eligible') return 'Eligible';
  if (kind === 'safe') return 'Safe';
  return label;
}

function guidanceLockedTarget(args: ItemCtx): string | null {
  if (args.target.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (isCustomPathTargetId(args.target.targetId) && args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return 'Unverified custom folder is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible, then review carefully.';
  if (args.target.risk === 'Unknown' && !args.allowDeleteUnknown) return 'Unknown item is locked by Settings. Enable "Allow deleting unknown folders" to make it eligible.';
  if (args.target.risk === 'Unknown' && isCustomPathTargetId(args.target.targetId)) return 'Unverified custom folder item. Review carefully; typed DELETE confirmation is required before cleaning.';
  if (args.target.risk === 'Unknown') return 'Unknown item. Review carefully; typed DELETE confirmation is required before cleaning.';
  return null;
}

function guidanceForRecent(minAgeHours?: number | null): string {
  if (minAgeHours === 0) return 'Recently modified. Sweeper skips recent files for safety under the current minimum age setting.';
  if (minAgeHours == null) return 'Modified within the minimum age window. Sweeper skips recent files for safety.';
  return `Modified within the minimum age window (${minAgeHours}h). Sweeper skips recent files for safety.`;
}

function guidanceByTargetName(args: ItemCtx): string | null {
  if (args.target.targetName === 'User Temp') return 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.';
  if (args.target.targetName === 'Crash Dumps & WER') return 'Local crash and error reports kept for diagnostics. Removing them does not affect app functionality.';
  if (args.target.targetName.toLowerCase().includes('npm')) return 'Package manager download cache. It can be redownloaded automatically when needed.';
  if (isCustomPathTargetId(args.target.targetId)) return 'Custom folder item. Only selected contents are cleaned and protected system directories remain excluded.';
  return null;
}

export function scanGuidance(args: ItemCtx & { selectable: boolean }): string {
  const locked = guidanceLockedTarget(args);
  if (locked) return locked;
  if (!args.item.ageEligible) return guidanceForRecent(args.minAgeHours);
  const byName = guidanceByTargetName(args);
  if (byName) return byName;
  if (args.target.risk === 'Caution') return 'Caution category. Review before removal; selected items are permanently deleted.';
  if (args.target.risk === 'Safe') return 'Safe to remove when not in active use. Selected items are permanently deleted after review and confirmation.';
  return 'Eligible under current safety settings. Selected items are permanently deleted after review and confirmation.';
}

export function customDisplayName(target: TargetScanResultDto): { title: string; tag: string | null } {
  if (target.targetName !== 'Custom Path (Unknown)') return { title: target.targetName, tag: null };
  const base = (resolvedRootPath(target).split('\\').filter(Boolean).pop()) || target.targetName;
  return { title: base, tag: 'Custom Path' };
}

export function visibleBaseTargets(targets: TargetScanResultDto[]): TargetScanResultDto[] {
  return targets.filter((t) => !(t.risk === 'Protected' && BigInt(t.totalBytes || '0') === 0n));
}

export function groupItemStatus(args: { target: TargetScanResultDto; items: ScanItemDto[]; selectableItems: ScanItemDto[]; selectedInTarget: number; isCustomUnknownLocked: boolean }) {
  const total = args.items.length; const eligible = args.selectableItems.length; const skipped = total - eligible;
  if (args.isCustomUnknownLocked) return { label: 'Skipped · Unverified', kind: 'skipped' as const, aria: 'Status Skipped' };
  if (args.target.risk === 'Protected') return { label: 'Protected', kind: 'protected' as const, aria: 'Status Protected' };
  if (eligible === 0 && total > 0) return { label: 'Skipped', kind: 'skipped' as const, aria: 'Status Skipped' };
  const riskKind = args.target.risk === 'Caution' ? 'caution' : args.target.risk === 'Unknown' ? 'unknown' : 'safe';
  const riskLabel = args.target.risk === 'Caution' ? 'Caution' : args.target.risk === 'Unknown' ? 'Unknown' : 'Safe';
  if (eligible > 0 && skipped === 0) return { label: riskLabel, kind: riskKind as ScanStatusKind, aria: `Status ${riskLabel}` };
  if (eligible > 0 && skipped > 0) return { label: eligible === 1 ? `1 ${riskLabel}` : `Partial ${riskLabel}`, kind: riskKind as ScanStatusKind, aria: `Status ${riskLabel}` };
  return null;
}

export function sortScanItems(args: { items: ScanItemDto[]; target: TargetScanResultDto; sort: ScanSort }): ScanItemDto[] {
  const next = [...args.items];
  if (args.sort === 'name') return next.sort((a, b) => relativeItemName({ itemPath: a.path, resolvedPaths: args.target.resolvedPaths }).localeCompare(relativeItemName({ itemPath: b.path, resolvedPaths: args.target.resolvedPaths })));
  return next.sort((a, b) => { const da = BigInt(a.size); const db = BigInt(b.size); return da === db ? 0 : (da > db ? -1 : 1); });
}

export function filterScanItems(args: { items: ScanItemDto[]; target: TargetScanResultDto; toolbar: ScanToolbarState; allowDeleteUnknown: boolean }): ScanItemDto[] {
  const q = args.toolbar.query.trim().toLowerCase();
  return args.items.filter((i) => {
    const selectable = isSelectableScanItem({ item: i, target: args.target, allowDeleteUnknown: args.allowDeleteUnknown });
    if (args.toolbar.filter === 'eligible' && !selectable) return false;
    if (args.toolbar.filter === 'skipped' && selectable) return false;
    if (!q) return true;
    const rel = relativeItemName({ itemPath: i.path, resolvedPaths: args.target.resolvedPaths }).toLowerCase();
    const name = rel.split(/[\\/]/).pop()?.toLowerCase() ?? '';
    return i.path.toLowerCase().includes(q) || rel.includes(q) || name.includes(q);
  });
}

export { formatBytes, middleTruncatePath, normalizeDisplayPath, sumDecimalStrings };
