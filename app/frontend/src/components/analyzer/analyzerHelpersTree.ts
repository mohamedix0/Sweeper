import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { AnalyzerStatusFilter, FlatAnalyzerRow } from './analyzerHelpersTypes';
import { analyzerStatusKey } from './analyzerHelpersStatus';
import { normalizedAnalyzerPath } from './analyzerHelpersUtils';

export const ANALYZER_CHILD_CAP = 25;
export function isMoreRowId(id: string): boolean { return id.startsWith('__more__:'); }
export function moreRowParentId(id: string): string { return id.slice('__more__:'.length); }

function cmpSizeDesc(args: { a: AnalyzerNodeDto; b: AnalyzerNodeDto }): number {
  const sa = BigInt(args.a.sizeBytes || '0');
  const sb = BigInt(args.b.sizeBytes || '0');
  if (sa === sb) return 0;
  if (sa > sb) return -1;
  return 1;
}
function bySize(a: AnalyzerNodeDto, b: AnalyzerNodeDto): number { return cmpSizeDesc({ a, b }); }
function visibleChildren(args: { children: AnalyzerNodeDto[]; showAll: boolean }) {
  if (args.showAll) return { kids: args.children, hidden: 0 };
  if (args.children.length <= ANALYZER_CHILD_CAP) return { kids: args.children, hidden: 0 };
  const bySizeList = [...args.children].sort(bySize).slice(0, ANALYZER_CHILD_CAP);
  const keep = new Set(bySizeList.map((k) => k.id));
  const kids = args.children.filter((k) => keep.has(k.id));
  return { kids, hidden: args.children.length - kids.length };
}
function nodeBytes(node: AnalyzerNodeDto): bigint { const raw = node.sizeBytes; if (raw) return BigInt(raw); return 0n; }
function bigger(a: bigint, b: bigint): bigint { if (b > a) return b; return a; }
function maxSiblingSize(kids: AnalyzerNodeDto[]): bigint {
  let max = 0n;
  for (const c of kids) max = bigger(max, nodeBytes(c));
  if (max === 0n) return 1n;
  return max;
}
function moreRow(args: { parent: AnalyzerNodeDto; hidden: number }): AnalyzerNodeDto {
  return {
    id: `__more__:${args.parent.id}`,
    name: `Showing top ${ANALYZER_CHILD_CAP} of ${args.parent.children.length} files · Show more`,
    path: '', isDir: false, sizeBytes: '0', itemCount: '0', risk: 'Safe', description: '', reason: null,
    tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: true, selectable: false, children: [],
  };
}
function pushRows(args: { rows: FlatAnalyzerRow[]; node: AnalyzerNodeDto; depth: number; siblingMax: bigint; expanded: Set<string>; showAllNodes: Set<string> }) {
  args.rows.push({ node: args.node, depth: args.depth, siblingMax: args.siblingMax });
  if (!args.node.isDir) return;
  if (!args.expanded.has(args.node.id)) return;
  const allKids = args.node.children ?? [];
  if (allKids.length === 0) return;
  const { kids, hidden } = visibleChildren({ children: allKids, showAll: args.showAllNodes.has(args.node.id) });
  const sibMax = maxSiblingSize(kids);
  const nextDepth = args.depth + 1;
  for (const c of kids) pushRows({ ...args, node: c, depth: nextDepth, siblingMax: sibMax });
  if (hidden <= 0) return;
  args.rows.push({ node: moreRow({ parent: args.node, hidden }), depth: nextDepth, siblingMax: 1n });
}
export function buildFlat(root: AnalyzerNodeDto, expanded: Set<string>, showAllNodes: Set<string>): FlatAnalyzerRow[] {
  const rows: FlatAnalyzerRow[] = [];
  const allKids = root.children ?? [];
  if (allKids.length === 0) return rows;
  const { kids, hidden } = visibleChildren({ children: allKids, showAll: showAllNodes.has(root.id) });
  const sibMax = maxSiblingSize(kids);
  for (const c of kids) pushRows({ rows, node: c, depth: 0, siblingMax: sibMax, expanded, showAllNodes });
  if (hidden > 0) rows.push({ node: moreRow({ parent: root, hidden }), depth: 0, siblingMax: 1n });
  return rows;
}

type SortMode = 'size' | 'items' | 'name';
type FilterCtx = { query: string; sort: SortMode; status: AnalyzerStatusFilter };
function itemCountValue(node: AnalyzerNodeDto): bigint { const raw = node.itemCount; if (raw) return BigInt(raw); return 0n; }
function sortByItems(children: AnalyzerNodeDto[]): AnalyzerNodeDto[] {
  return [...children].sort((a, b) => Number(itemCountValue(b) - itemCountValue(a)));
}
function sortChildren(children: AnalyzerNodeDto[], sort: SortMode): AnalyzerNodeDto[] {
  if (sort === 'name') return [...children].sort((a, b) => a.name.localeCompare(b.name));
  if (sort === 'items') return sortByItems(children);
  return [...children].sort(bySize);
}
function matchesQuery(node: AnalyzerNodeDto, q: string): boolean {
  if (!q) return true;
  if (node.name.toLowerCase().includes(q)) return true;
  return node.path.toLowerCase().includes(q);
}
function statusMatches(node: AnalyzerNodeDto, filter: AnalyzerStatusFilter): boolean {
  if (filter === 'all') return true;
  return analyzerStatusKey({ node }) === filter;
}
function filterChildren(node: AnalyzerNodeDto, ctx: FilterCtx): AnalyzerNodeDto[] {
  const filtered = (node.children ?? []).map((c) => filterTreeNode(c, ctx)).filter(Boolean) as AnalyzerNodeDto[];
  return sortChildren(filtered, ctx.sort);
}
function chooseFilteredNode(node: AnalyzerNodeDto, children: AnalyzerNodeDto[], selfMatch: boolean): AnalyzerNodeDto | null {
  if (selfMatch) return { ...node, children };
  if (children.length === 0) return null;
  return { ...node, children };
}
function filterTreeNode(node: AnalyzerNodeDto, ctx: FilterCtx): AnalyzerNodeDto | null {
  const children = filterChildren(node, ctx);
  let selfMatch = false;
  if (matchesQuery(node, ctx.query)) selfMatch = statusMatches(node, ctx.status);
  return chooseFilteredNode(node, children, selfMatch);
}
export function filterTree(node: AnalyzerNodeDto, query: string, sort: SortMode, statusFilter: AnalyzerStatusFilter): AnalyzerNodeDto | null {
  return filterTreeNode(node, { query: query.trim().toLowerCase(), sort, status: statusFilter });
}
function needsConfirmNode(n: AnalyzerNodeDto): boolean { if (n.risk === 'Unknown') return true; return !n.targetId; }
export function analyzerSelectionNeedsConfirmation(nodes: AnalyzerNodeDto[]): boolean { return nodes.some((n) => needsConfirmNode(n)); }
export function isAncestorAnalyzerPath(ancestorPath: string, descendantPath: string): boolean {
  const a = normalizedAnalyzerPath({ path: ancestorPath });
  const d = normalizedAnalyzerPath({ path: descendantPath });
  if (!a) return false;
  if (!d) return false;
  if (a === d) return false;
  let prefix = a;
  if (!a.endsWith('\\')) prefix = `${a}\\`;
  return d.startsWith(prefix);
}
export function dedupeAnalyzerNodesByCoverage(nodes: AnalyzerNodeDto[]): AnalyzerNodeDto[] {
  return nodes.filter((node, _, all) => !all.some((other) => {
    if (other.id === node.id) return false;
    return isAncestorAnalyzerPath(other.path, node.path);
  }));
}
