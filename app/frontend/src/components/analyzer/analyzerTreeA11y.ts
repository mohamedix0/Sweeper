/**
 * Single source of truth for Analyzer tree DOM ids and the Show more / Show
 * fewer pseudo-row id scheme. Both building and classifying these ids live
 * here so the prefixes and slices are never duplicated across modules.
 */
export type AnalyzerPseudoRowId =
  | { readonly kind: 'more'; readonly parentId: string }
  | { readonly kind: 'fewer'; readonly parentId: string };

const MORE_PREFIX = '__more__:';
const FEWER_PREFIX = '__fewer__:';

export function moreRowId(parentId: string): string {
  return `${MORE_PREFIX}${parentId}`;
}

export function fewerRowId(parentId: string): string {
  return `${FEWER_PREFIX}${parentId}`;
}

export function parseAnalyzerRowId(id: string): AnalyzerPseudoRowId | null {
  if (id.startsWith(MORE_PREFIX)) return { kind: 'more', parentId: id.slice(MORE_PREFIX.length) };
  if (id.startsWith(FEWER_PREFIX)) return { kind: 'fewer', parentId: id.slice(FEWER_PREFIX.length) };
  return null;
}

export function isMoreRowId(id: string): boolean {
  return parseAnalyzerRowId(id)?.kind === 'more';
}

export function isFewerRowId(id: string): boolean {
  return parseAnalyzerRowId(id)?.kind === 'fewer';
}

export function showAllRowParentId(id: string): string {
  return parseAnalyzerRowId(id)?.parentId ?? id;
}

export function treeItemDomId(nodeId: string): string {
  return `analyzer-tree-item-${nodeId}`;
}

export function showAllButtonDomId(parentId: string): string {
  return `analyzer-show-all-${parentId}`;
}

/** DOM id of the single role=tree tab stop that owns DOM focus. */
export const ANALYZER_TREE_DOM_ID = 'analyzer-results-tree';

/** Marks every interactive descendant control of a tree row. */
export const TREE_CONTROL_ATTR = 'data-tree-control';

export type TreeControlKind = 'expand' | 'select' | 'drill' | 'showmore';

/** Stable id for a row control, used for programmatic focus. */
export function treeControlDomId(nodeId: string, kind: TreeControlKind): string {
  return `${treeItemDomId(nodeId)}--control-${kind}`;
}
