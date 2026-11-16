import { useMemo } from 'react';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { StartAnalyzerResult } from '../../api/client';
import { analyzerSelectionNeedsConfirmation, analyzerStatusKey, buildFlat, dedupeAnalyzerNodesByCoverage, filterTree, type AnalyzerStatusFilter, type FlatAnalyzerRow } from './analyzerHelpers';

type Args = {
  result: StartAnalyzerResult | null;
  query: string;
  sort: 'size' | 'items' | 'name';
  statusFilter: AnalyzerStatusFilter;
  expanded: Set<string>;
  showAllAnalyzerNodes: Set<string>;
  selectedIds: Set<string>;
};

function walkNodes(root: AnalyzerNodeDto, map: Map<string, AnalyzerNodeDto>) {
  map.set(root.id, root);
  for (const c of root.children ?? []) walkNodes(c, map);
}

function bumpCount(counts: { safe: number; caution: number; unknown: number; locked: number }, key: string) {
  if (key === 'safe') { counts.safe += 1; return; }
  if (key === 'caution') { counts.caution += 1; return; }
  if (key === 'unknown') { counts.unknown += 1; return; }
  counts.locked += 1;
}

function statusCountsOf(result: StartAnalyzerResult | null) {
  const counts = { all: 0, safe: 0, caution: 0, unknown: 0, locked: 0 };
  if (!result) return counts;
  const kids = result.root.children ?? [];
  counts.all = kids.length;
  for (const n of kids) bumpCount(counts, analyzerStatusKey({ node: n }));
  return counts;
}

function selectedNodesOf(args: { selectedIds: Set<string>; nodesById: Map<string, AnalyzerNodeDto> }) {
  return Array.from(args.selectedIds).map((id) => args.nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
}

function bytesOf(nodes: AnalyzerNodeDto[]): string {
  return nodes.reduce((a, n) => a + BigInt(n.sizeBytes || '0'), 0n).toString();
}

function totalBytesRaw(result: StartAnalyzerResult | null): string {
  const rootSize = result?.root.sizeBytes;
  if (rootSize) return rootSize;
  const total = result?.totalBytes;
  if (total) return total;
  return '0';
}

function totalBytesOf(result: StartAnalyzerResult | null): bigint {
  const value = BigInt(totalBytesRaw(result));
  if (value === 0n) return 1n;
  return value;
}

export function useAnalyzerDerived(args: Args) {
  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    if (args.result?.root) walkNodes(args.result.root, map);
    return map;
  }, [args.result]);

  const filteredRoot = useMemo(() => {
    if (!args.result) return null;
    return filterTree(args.result.root, args.query, args.sort, args.statusFilter);
  }, [args.result, args.query, args.sort, args.statusFilter]);

  const flatRows = useMemo(() => {
    if (!filteredRoot) return [] as FlatAnalyzerRow[];
    return buildFlat(filteredRoot, args.expanded, args.showAllAnalyzerNodes);
  }, [filteredRoot, args.expanded, args.showAllAnalyzerNodes]);

  const statusCounts = useMemo(() => statusCountsOf(args.result), [args.result]);
  const selectedNodes = useMemo(() => selectedNodesOf({ selectedIds: args.selectedIds, nodesById }), [args.selectedIds, nodesById]);
  const dedupedSelectedNodes = useMemo(() => dedupeAnalyzerNodesByCoverage(selectedNodes), [selectedNodes]);
  const selectedBytes = useMemo(() => bytesOf(dedupedSelectedNodes), [dedupedSelectedNodes]);
  const requiresConfirmation = useMemo(() => analyzerSelectionNeedsConfirmation(dedupedSelectedNodes), [dedupedSelectedNodes]);
  const rootTotalBytes = useMemo(() => totalBytesOf(args.result), [args.result]);
  return { nodesById, filteredRoot, flatRows, statusCounts, selectedNodes, dedupedSelectedNodes, selectedBytes, requiresConfirmation, rootTotalBytes };
}
