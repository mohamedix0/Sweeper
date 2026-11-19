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
