import type { AnalyzerNodeDto } from '../../contract/bindings';
import { isAncestorAnalyzerPath } from './analyzerHelpers';

export function toggleExpandedId(prev: Set<string>, node: AnalyzerNodeDto): Set<string> {
  if (!node.isDir) return prev;
  const children = node.children ?? [];
  if (children.length === 0) return prev;
  const next = new Set(prev);
  if (next.has(node.id)) next.delete(node.id);
  else next.add(node.id);
  return next;
}

function maybeDeleteDescendant(args: { id: string; node: AnalyzerNodeDto; nodesById: Map<string, AnalyzerNodeDto>; next: Set<string> }) {
  const existing = args.nodesById.get(args.id);
  if (!existing) return;
  if (isAncestorAnalyzerPath(args.node.path, existing.path)) args.next.delete(args.id);
}

function removeDescendants(next: Set<string>, node: AnalyzerNodeDto, nodesById: Map<string, AnalyzerNodeDto>) {
  Array.from(next).forEach((id) => maybeDeleteDescendant({ id, node, nodesById, next }));
}

function matchesSelectedAncestor(args: { id: string; node: AnalyzerNodeDto; nodesById: Map<string, AnalyzerNodeDto> }): boolean {
  const existing = args.nodesById.get(args.id);
  if (!existing) return false;
  return isAncestorAnalyzerPath(existing.path, args.node.path);
}

function hasSelectedAncestor(next: Set<string>, node: AnalyzerNodeDto, nodesById: Map<string, AnalyzerNodeDto>) {
  return Array.from(next).some((id) => matchesSelectedAncestor({ id, node, nodesById }));
}

export function nextSelectionAfterToggle(args: { prev: Set<string>; node: AnalyzerNodeDto; nodesById: Map<string, AnalyzerNodeDto> }): Set<string> {
  const next = new Set(args.prev);
  if (next.has(args.node.id)) {
    next.delete(args.node.id);
    return next;
  }
  removeDescendants(next, args.node, args.nodesById);
  if (!hasSelectedAncestor(next, args.node, args.nodesById)) next.add(args.node.id);
  return next;
}

function hasCoveredAncestor(merged: AnalyzerNodeDto[], node: AnalyzerNodeDto): boolean {
  return merged.some((m) => m.id === node.id || isAncestorAnalyzerPath(m.path, node.path));
}

function dropCoveredAt(args: { merged: AnalyzerNodeDto[]; node: AnalyzerNodeDto; index: number }) {
  if (!isAncestorAnalyzerPath(args.node.path, args.merged[args.index].path)) return;
  args.merged.splice(args.index, 1);
}

function removeCoveredDescendants(merged: AnalyzerNodeDto[], node: AnalyzerNodeDto) {
  let i = merged.length - 1;
  while (i >= 0) {
    dropCoveredAt({ merged, node, index: i });
    i -= 1;
  }
}

function applySafeAddition(merged: AnalyzerNodeDto[], node: AnalyzerNodeDto) {
  if (hasCoveredAncestor(merged, node)) return;
  removeCoveredDescendants(merged, node);
  merged.push(node);
}

export function mergeSafeSelection(args: { prev: Set<string>; additions: AnalyzerNodeDto[]; nodesById: Map<string, AnalyzerNodeDto> }): Set<string> {
  const merged = Array.from(args.prev).map((id) => args.nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
  args.additions.forEach((node) => applySafeAddition(merged, node));
  return new Set(merged.map((n) => n.id));
}

export function expandAncestorsForNode(args: { root: AnalyzerNodeDto; nodeId: string }): Set<string> {
  const next = new Set<string>();
  function visit(node: AnalyzerNodeDto, ancestors: string[]): boolean {
    if (node.id === args.nodeId) {
      ancestors.forEach((a) => next.add(a));
      return true;
    }
    return (node.children ?? []).some((c) => visit(c, [...ancestors, node.id]));
  }
  visit(args.root, []);
  return next;
}
