import type { AnalyzerNodeDto } from '../../contract/bindings';
import { normalizeDisplayPath } from '../../utils/format';

export type FlatAnalyzerRow = { node: AnalyzerNodeDto; depth: number; siblingMax: bigint };
export type AnalyzerStatusFilter = 'all' | 'safe' | 'caution' | 'unknown' | 'locked';
export const ANALYZER_CHILD_CAP = 25;

export function isMoreRowId(id: string): boolean { return id.startsWith('__more__:'); }
export function moreRowParentId(id: string): string { return id.slice('__more__:'.length); }

function visibleChildren(args: { children: AnalyzerNodeDto[]; showAll: boolean }) {
  if (args.showAll || args.children.length <= ANALYZER_CHILD_CAP) return { kids: args.children, hidden: 0 };
  const bySize = [...args.children].sort((a, b) => {
    const sa = BigInt(a.sizeBytes || '0'); const sb = BigInt(b.sizeBytes || '0');
    return sa === sb ? 0 : (sa > sb ? -1 : 1);
  }).slice(0, ANALYZER_CHILD_CAP);
  const keep = new Set(bySize.map((k) => k.id));
  const kids = args.children.filter((k) => keep.has(k.id));
  return { kids, hidden: args.children.length - kids.length };
}

function maxSiblingSize(kids: AnalyzerNodeDto[]): bigint {
  let max = 0n;
  for (const c of kids) { const s = BigInt(c.sizeBytes || '0'); if (s > max) max = s; }
  return max === 0n ? 1n : max;
}

function moreRow(parent: AnalyzerNodeDto, hidden: number): AnalyzerNodeDto {
  return {
    id: `__more__:${parent.id}`,
    name: `Showing top ${ANALYZER_CHILD_CAP} of ${parent.children.length} files · Show more`,
    path: '', isDir: false, sizeBytes: '0', itemCount: '0', risk: 'Safe', description: '', reason: null,
    tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: true, selectable: false, children: [],
  };
}

export function buildFlat(root: AnalyzerNodeDto, expanded: Set<string>, showAllNodes: Set<string>): FlatAnalyzerRow[] {
  const rows: FlatAnalyzerRow[] = [];
  function walk(node: AnalyzerNodeDto, depth: number, siblingMax: bigint) {
    rows.push({ node, depth, siblingMax });
    if (!node.isDir || !expanded.has(node.id) || !node.children?.length) return;
    const allKids = node.children;
    const { kids, hidden } = visibleChildren({ children: allKids, showAll: showAllNodes.has(node.id) });
    const sibMax = maxSiblingSize(kids);
    for (const c of kids) walk(c, depth + 1, sibMax);
    if (hidden > 0) rows.push({ node: moreRow(node, hidden), depth: depth + 1, siblingMax: 1n });
  }
  const allKids = root.children ?? [];
  if (allKids.length === 0) return rows;
  const { kids, hidden } = visibleChildren({ children: allKids, showAll: showAllNodes.has(root.id) });
  const sibMax = maxSiblingSize(kids);
  for (const c of kids) walk(c, 0, sibMax);
  if (hidden > 0) rows.push({ node: moreRow(root, hidden), depth: 0, siblingMax: 1n });
  return rows;
}

type SortMode = 'size' | 'items' | 'name';
type FilterCtx = { query: string; sort: SortMode; status: AnalyzerStatusFilter };

function bySize(a: AnalyzerNodeDto, b: AnalyzerNodeDto): number {
  const sa = BigInt(a.sizeBytes || '0');
  const sb = BigInt(b.sizeBytes || '0');
  if (sa === sb) return 0;
  return sa > sb ? -1 : 1;
}

function sortChildren(children: AnalyzerNodeDto[], sort: SortMode): AnalyzerNodeDto[] {
  if (sort === 'name') return [...children].sort((a, b) => a.name.localeCompare(b.name));
  if (sort === 'items') return [...children].sort((a, b) => Number(BigInt(b.itemCount || '0') - BigInt(a.itemCount || '0')));
  return [...children].sort(bySize);
}

function matchesQuery(node: AnalyzerNodeDto, q: string): boolean {
  if (!q) return true;
  const name = node.name.toLowerCase();
  if (name.includes(q)) return true;
  return node.path.toLowerCase().includes(q);
}

function statusMatches(node: AnalyzerNodeDto, filter: AnalyzerStatusFilter): boolean {
  if (filter === 'all') return true;
  return analyzerStatusKey(node) === filter;
}

function filterChildren(node: AnalyzerNodeDto, ctx: FilterCtx): AnalyzerNodeDto[] {
  const filtered = (node.children ?? []).map((c) => filterTreeNode(c, ctx)).filter(Boolean) as AnalyzerNodeDto[];
  return sortChildren(filtered, ctx.sort);
}

function filterTreeNode(node: AnalyzerNodeDto, ctx: FilterCtx): AnalyzerNodeDto | null {
  const children = filterChildren(node, ctx);
  const selfMatch = matchesQuery(node, ctx.query) && statusMatches(node, ctx.status);
  if (selfMatch) return { ...node, children };
  if (children.length === 0) return null;
  return { ...node, children };
}

export function filterTree(node: AnalyzerNodeDto, query: string, sort: SortMode, statusFilter: AnalyzerStatusFilter): AnalyzerNodeDto | null {
  return filterTreeNode(node, { query: query.trim().toLowerCase(), sort, status: statusFilter });
}

export function analyzerSelectionNeedsConfirmation(nodes: AnalyzerNodeDto[]): boolean {
  return nodes.some((n) => n.risk === 'Unknown' || !n.targetId);
}

export function isVhdxPath(path: string): boolean { return path.toLowerCase().endsWith('.vhdx'); }

export function isNodeSelectable(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): boolean {
  if (node.displayOnly || node.isLinkPoint || node.isCloudPlaceholder) return false;
  if (node.risk === 'Protected') return false;
  if (isVhdxPath(node.path)) return false;
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return !!node.selectable;
}

export function isCleanableEligible(node: AnalyzerNodeDto, selectable: boolean): boolean {
  if (!selectable) return false;
  const tier = node.tier;
  return tier === 'Tier1' || tier === 'Tier2' || tier === 'Tier3';
}

export function isDriveRoot(node: AnalyzerNodeDto): boolean {
  return /^[A-Za-z]:\\?$/.test(node.path.trim()) || /^[A-Za-z]:\\$/.test(normalizeDisplayPath(node.path));
}

type Pill = { label: string; className: string } | null;
const PILL_PROTECTED = { label: 'Protected', className: 'status-pill status-pill-protected' };
const PILL_READONLY = { label: 'Read-Only', className: 'status-pill status-pill-readonly' };
const PILL_DRIVE = { label: 'Drive Root', className: 'status-pill status-pill-readonly' };
const PILL_UNKNOWN = { label: 'Unknown', className: 'status-pill status-pill-unknown' };
const PILL_CAUTION = { label: 'Caution', className: 'status-pill status-pill-caution' };
const PILL_SAFE = { label: 'Safe', className: 'status-pill status-pill-safe' };

export function analyzerStatusPill(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): Pill {
  void allowDeleteUnknown;
  if (isDriveRoot(node)) return PILL_DRIVE;
  if (node.risk === 'Protected') return PILL_PROTECTED;
  if (isVhdxPath(node.path) || node.isLinkPoint || node.isCloudPlaceholder || node.displayOnly) return PILL_READONLY;
  if (node.risk === 'Unknown') return PILL_UNKNOWN;
  if (node.risk === 'Caution') return PILL_CAUTION;
  if (isCleanableEligible(node, selectable)) return PILL_SAFE;
  if (selectable) return PILL_SAFE;
  return null;
}

export function analyzerStatusKey(node: AnalyzerNodeDto): AnalyzerStatusFilter {
  if (isDriveRoot(node)) return 'locked';
  if (node.risk === 'Protected') return 'locked';
  if (isVhdxPath(node.path) || node.isLinkPoint || node.isCloudPlaceholder || node.displayOnly) return 'locked';
  if (node.risk === 'Unknown') return 'unknown';
  if (node.risk === 'Caution') return 'caution';
  if (node.risk === 'Safe') return 'safe';
  return 'locked';
}

export function normalizedAnalyzerPath(path: string): string {
  return normalizeDisplayPath(path).replace(/[\\/]+$/, '').toLowerCase();
}

export function isAncestorAnalyzerPath(ancestorPath: string, descendantPath: string): boolean {
  const a = normalizedAnalyzerPath(ancestorPath);
  const d = normalizedAnalyzerPath(descendantPath);
  if (!a || !d || a === d) return false;
  const prefix = a.endsWith('\\') ? a : `${a}\\`;
  return d.startsWith(prefix);
}

export function dedupeAnalyzerNodesByCoverage(nodes: AnalyzerNodeDto[]): AnalyzerNodeDto[] {
  return nodes.filter((node, _, all) => !all.some((other) => other.id !== node.id && isAncestorAnalyzerPath(other.path, node.path)));
}

function displayOnlyReason(node: AnalyzerNodeDto): string {
  if (isDriveRoot(node)) return 'Drive roots cannot be cleaned as a whole. Expand to select specific folders.';
  return 'Display-only. Not selectable.';
}

function lockedReason(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): { text: string; unknownGated: boolean } {
  if (node.risk === 'Protected') return { text: 'Protected. Not selectable.', unknownGated: false };
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return { text: 'Unknown gated by Settings (Allow deleting unknown folders).', unknownGated: true };
  if (node.isLinkPoint) return { text: 'Link Point. Not selectable.', unknownGated: false };
  if (node.isCloudPlaceholder) return { text: 'Cloud placeholder (Skipped). Not selectable.', unknownGated: false };
  if (isVhdxPath(node.path)) return { text: 'Read-only virtual disk. Not selectable.', unknownGated: false };
  if (node.displayOnly) return { text: displayOnlyReason(node), unknownGated: false };
  if (node.reason) return { text: `${node.reason}. Not selectable.`, unknownGated: false };
  return { text: 'Not selectable.', unknownGated: false };
}

export function analyzerStatusText(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): { text: string; locked: boolean; unknownGated: boolean } {
  if (selectable) {
    const needsConfirm = node.risk === 'Unknown' || !node.targetId;
    return { text: needsConfirm ? 'Selectable (typed confirmation required)' : 'Meets age limit (> 24h old)', locked: false, unknownGated: false };
  }
  const locked = lockedReason(node, allowDeleteUnknown);
  return { text: locked.text, locked: true, unknownGated: locked.unknownGated };
}

function unknownGuidance(allowDeleteUnknown: boolean): string {
  if (!allowDeleteUnknown) return 'Unknown folder is locked by Settings. Enable "Allow deleting unknown folders" to allow cleanup after typed confirmation.';
  return 'Unknown folder. Review carefully; typed DELETE confirmation is required before cleaning.';
}

function guidanceLocked(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): string | null {
  if (isVhdxPath(node.path)) return 'Virtual disk (WSL/Hyper-V). Read-only in Sweeper — compaction is manual and safe to do when the VM/WSL is shut down.';
  if (node.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (node.isCloudPlaceholder) return 'Cloud placeholder file. Read-only in Analyzer; content may be remote and is not removable here.';
  if (node.isLinkPoint) return 'Link point (symlink/junction). Read-only in Analyzer to avoid traversing or removing linked targets.';
  if (node.risk === 'Unknown') return unknownGuidance(allowDeleteUnknown);
  if (isDriveRoot(node)) return 'Drive roots cannot be cleaned as a whole. Expand C: to select specific folders.';
  return null;
}

export function analyzerGuidance(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): string {
  const locked = guidanceLocked(node, allowDeleteUnknown);
  if (locked) return locked;
  if (selectable && isCleanableEligible(node, selectable)) return 'Eligible cache/build artifact under current settings. Selected items are permanently deleted after review and confirmation.';
  if (selectable) return 'Eligible under current settings. Selected items are permanently deleted after review and confirmation.';
  return 'Display-only item. It cannot be selected for cleanup in Analyzer.';
}

export function nodeTypeTag(node: AnalyzerNodeDto): string | null {
  if (isVhdxPath(node.path)) return 'Virtual Disk';
  if (node.isLinkPoint) return 'Symlink';
  if (node.isCloudPlaceholder) return 'Cloud';
  return null;
}

export const ALLOC_COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#8b5cf6', '#ef4444', '#64748b'];
export const BROWSE_PATHS = ['C:\\', 'C:\\Users', 'C:\\Windows\\Temp'];

function shareRatio(nodeBytes: bigint, rootTotalBytes: bigint): number {
  if (nodeBytes <= 0n || rootTotalBytes <= 0n) return 0;
  return Number(nodeBytes) / Number(rootTotalBytes);
}

export function shareBarFillPx(nodeBytes: bigint, rootTotalBytes: bigint): number {
  const ratio = shareRatio(nodeBytes, rootTotalBytes);
  if (ratio * 100 < 0.1) return 0;
  const width = Math.round(Math.sqrt(Math.max(0, ratio)) * 48);
  return Math.max(2, Math.min(48, width));
}

export function formatSharePercent(share: number): string {
  if (share <= 0) return '0%';
  if (share < 0.1) return '<0.1%';
  const pct1 = Math.floor(share * 10) / 10;
  return `${pct1.toFixed(1)}%`;
}

export function escapeVhdxPathForPowerShell(path: string): string {
  const vhdxPath = normalizeDisplayPath(path);
  return `'${vhdxPath.replace(/['\u2018\u2019\u201A\u201B]/g, (q) => q + q)}'`;
}
