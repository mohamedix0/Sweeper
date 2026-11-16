import { useMemo, useRef, useState } from 'react';
import type { CleanProgressEvent, ExecuteCleanResponse, Settings, SweeperClient } from '../../api/client';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { PreviewResult } from '../cleanShared';
import { BROWSE_PATHS, analyzerStatusKey, dedupeAnalyzerNodesByCoverage, isNodeSelectable, type AnalyzerStatusFilter } from './analyzerHelpers';
import { runAnalyzerAction } from './analyzerWorkflowRun';
import { runCleanAction } from './analyzerWorkflowClean';
import { runPreviewAction } from './analyzerWorkflowPreview';
import { expandAncestorsForNode, mergeSafeSelection, nextSelectionAfterToggle, toggleExpandedId } from './analyzerWorkflowUtils';
import { useAnalyzerDerived } from './useAnalyzerDerived';
import { useAnalyzerSession } from './useAnalyzerSession';

type WfArgs = { client: SweeperClient; settings: Settings | null };
type S = ReturnType<typeof useAnalyzerSession>;
type D = ReturnType<typeof useAnalyzerDerived>;
type Msg = { error: string | null; info: string | null };

function useMessages() {
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  return { error, setError, info, setInfo };
}

function usePreviewState() {
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const rev = useRef(0);
  const active = useRef(0);
  return { preview, setPreview, previewLoading, setPreviewLoading, confirmText, setConfirmText, rev, active };
}

function useCleanState() {
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  return { cleaning, setCleaning, cleanProgress, setCleanProgress, cleanSummary, setCleanSummary };
}

function bumpPreviewRefs(input: { rev: { current: number }; active: { current: number } }) {
  input.rev.current += 1;
  input.active.current += 1;
}

function clearPreview(input: { p: ReturnType<typeof usePreviewState>; c: ReturnType<typeof useCleanState> }) {
  input.p.setPreview(null);
  input.c.setCleanSummary(null);
  input.c.setCleanProgress(null);
}

function nextBrowsePath(input: { current: string }) {
  const idx = BROWSE_PATHS.findIndex((p: string) => p.toLowerCase() === input.current.trim().toLowerCase());
  return BROWSE_PATHS[(idx + 1 + BROWSE_PATHS.length) % BROWSE_PATHS.length];
}

function collectSafeNodes(input: { d: D; allowDeleteUnknown: boolean }) {
  const safeNodes: AnalyzerNodeDto[] = [];
  input.d.nodesById.forEach((node) => {
    const allowed = isNodeSelectable({ node, allowDeleteUnknown: input.allowDeleteUnknown });
    if (!allowed) return;
    const key = analyzerStatusKey({ node });
    if (key === 'safe') safeNodes.push(node);
  });
  return dedupeAnalyzerNodesByCoverage(safeNodes);
}

function useSafeNodes(input: { d: D; allowDeleteUnknown: boolean }) {
  return useMemo(() => collectSafeNodes(input), [input]);
}

function toggleExpandAll(input: { s: S; ids: string[] }) {
  if (!input.s.result) return;
  const allOpen = input.ids.length > 0 && input.ids.every((id) => input.s.expanded.has(id));
  if (allOpen) {
    input.s.setExpanded(new Set());
    return;
  }
  input.s.setExpanded(new Set(input.ids));
}

function focusNodeInTree(input: { s: S; d: D; id: string; inspect: (id: string) => void }) {
  const node = input.d.nodesById.get(input.id);
  if (!node) return;
  if (input.s.result) {
    const ancestors = expandAncestorsForNode({ root: input.s.result.root, nodeId: input.id });
    input.s.setExpanded((prev) => new Set([...prev, ...ancestors]));
  }
  input.inspect(input.id);
}

function resetSession(input: { s: S; p: ReturnType<typeof usePreviewState>; c: ReturnType<typeof useCleanState>; m: ReturnType<typeof useMessages>; msg?: string; invalidate: () => void }) {
  input.invalidate();
  input.s.resetInspection();
  input.s.setSessionId(null);
  input.s.setStartedAt(null);
  input.s.setResult(null);
  input.s.setSelectedIds(new Set());
  input.s.setExpanded(new Set());
  input.s.setShowAllAnalyzerNodes(new Set());
  input.p.setPreview(null);
  input.p.setConfirmText('');
  input.c.setCleanProgress(null);
  if (input.msg) input.m.setInfo(input.msg);
}

function toggleSelectNode(input: { s: S; d: D; p: ReturnType<typeof usePreviewState>; c: ReturnType<typeof useCleanState>; node: AnalyzerNodeDto; allowDeleteUnknown: boolean; inspect: (id: string) => void; invalidate: () => void }) {
  const selectable = isNodeSelectable({ node: input.node, allowDeleteUnknown: input.allowDeleteUnknown });
  if (!selectable) {
    input.inspect(input.node.id);
    return;
  }
  input.invalidate();
  clearPreview({ p: input.p, c: input.c });
  input.s.setSelectedIds((prev) => nextSelectionAfterToggle({ prev, node: input.node, nodesById: input.d.nodesById }));
  input.inspect(input.node.id);
}

function selectSafe(input: { s: S; d: D; p: ReturnType<typeof usePreviewState>; c: ReturnType<typeof useCleanState>; additions: AnalyzerNodeDto[]; invalidate: () => void }) {
  input.invalidate();
  clearPreview({ p: input.p, c: input.c });
  input.s.setSelectedIds((prev) => mergeSafeSelection({ prev, additions: input.additions, nodesById: input.d.nodesById }));
}

type WorkflowApiInput = { s: S; d: D; m: ReturnType<typeof useMessages>; p: ReturnType<typeof usePreviewState>; c: ReturnType<typeof useCleanState>; args: WfArgs; query: string; setQuery: (v: string) => void; sort: 'size' | 'items' | 'name'; setSort: (v: 'size' | 'items' | 'name') => void; statusFilter: AnalyzerStatusFilter; setStatusFilter: (v: AnalyzerStatusFilter) => void; allowDeleteUnknown: boolean; safeSelectableNodes: AnalyzerNodeDto[] };

function buildWorkflowApi(i: WorkflowApiInput) {
  const invalidatePreviewRequest = () => { bumpPreviewRefs({ rev: i.p.rev, active: i.p.active }); i.p.setPreviewLoading(false); };
  const clearRunState = () => { i.m.setError(null); i.m.setInfo(null); clearPreview({ p: i.p, c: i.c }); };
  const inspectNode = (id: string) => { void i.s.inspectNode({ itemId: id }); };
  const resetAnalyzerSession = (msg?: string) => resetSession({ s: i.s, p: i.p, c: i.c, m: i.m, msg, invalidate: invalidatePreviewRequest });
  const runAnalyzer = async (input?: { path?: string }) => runAnalyzerAction({ client: i.args.client, pathOverride: input?.path, rootPath: i.s.rootPath, setRootPath: i.s.setRootPath, invalidatePreviewRequest, resetInspection: i.s.resetInspection, clearRunState, setSelectedIds: i.s.setSelectedIds, setResult: i.s.setResult, setExpanded: i.s.setExpanded, setShowAllAnalyzerNodes: i.s.setShowAllAnalyzerNodes, setAnalyzing: i.s.setAnalyzing, setProgress: i.s.setProgress, setSessionId: i.s.setSessionId, setStartedAt: i.s.setStartedAt, setAnalyzedPath: i.s.setAnalyzedPath, inspectNode: ({ id }) => inspectNode(id), setError: i.m.setError });
  const toggleSelect = (node: AnalyzerNodeDto) => toggleSelectNode({ s: i.s, d: i.d, p: i.p, c: i.c, node, allowDeleteUnknown: i.allowDeleteUnknown, inspect: inspectNode, invalidate: invalidatePreviewRequest });
  const selectSafeAnalyzer = () => selectSafe({ s: i.s, d: i.d, p: i.p, c: i.c, additions: i.safeSelectableNodes, invalidate: invalidatePreviewRequest });
  const runPreview = async () => runPreviewAction({ client: i.args.client, sessionId: i.s.sessionId, dedupedSelectedNodes: i.d.dedupedSelectedNodes, requiresConfirmation: i.d.requiresConfirmation, confirmText: i.p.confirmText, refs: { previewRevision: i.p.rev, activePreviewRequest: i.p.active }, setError: i.m.setError, setInfo: i.m.setInfo, setPreviewLoading: i.p.setPreviewLoading, setPreview: i.p.setPreview, setCleanSummary: i.c.setCleanSummary, setCleanProgress: i.c.setCleanProgress, resetAnalyzerSession });
  const runClean = async () => runCleanAction({ client: i.args.client, preview: i.p.preview, setError: i.m.setError, setInfo: i.m.setInfo, setCleaning: i.c.setCleaning, setCleanProgress: i.c.setCleanProgress, setCleanSummary: i.c.setCleanSummary, setPreview: i.p.setPreview, setSelectedIds: i.s.setSelectedIds, setConfirmText: i.p.setConfirmText, resetAnalyzerSession });
  return { invalidatePreviewRequest, inspectNode, runAnalyzer, toggleSelect, selectSafeAnalyzer, runPreview, runClean, resetAnalyzerSession };
}

export function useAnalyzerWorkflow(args: WfArgs) {
  const s = useAnalyzerSession({ client: args.client });
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'size' | 'items' | 'name'>('size');
  const [statusFilter, setStatusFilter] = useState<AnalyzerStatusFilter>('all');
  const m = useMessages();
  const p = usePreviewState();
  const c = useCleanState();
  const allowDeleteUnknown = args.settings?.allowDeleteUnknown ?? false;
  const d = useAnalyzerDerived({ result: s.result, query, sort, statusFilter, expanded: s.expanded, showAllAnalyzerNodes: s.showAllAnalyzerNodes, selectedIds: s.selectedIds });
  const safeSelectableNodes = useSafeNodes({ d, allowDeleteUnknown });
  const api = buildWorkflowApi({ s, d, m, p, c, args, query, setQuery, sort, setSort, statusFilter, setStatusFilter, allowDeleteUnknown, safeSelectableNodes });
  return {
    rootPath: s.rootPath, setRootPath: s.setRootPath, analyzedPath: s.analyzedPath, analyzing: s.analyzing, progress: s.progress,
    sessionId: s.sessionId, result: s.result, expanded: s.expanded, selectedIds: s.selectedIds, inspected: s.inspected,
    preview: p.preview, previewLoading: p.previewLoading, confirmText: p.confirmText, setConfirmText: p.setConfirmText,
    cleaning: c.cleaning, cleanProgress: c.cleanProgress, cleanSummary: c.cleanSummary, error: m.error, info: m.info,
    query, setQuery, sort, setSort, statusFilter, setStatusFilter,
    showAllAnalyzerNodes: s.showAllAnalyzerNodes, setShowAllAnalyzerNodes: s.setShowAllAnalyzerNodes,
    nodesById: d.nodesById, filteredRoot: d.filteredRoot, flatRows: d.flatRows, statusCounts: d.statusCounts,
    selectedNodes: d.selectedNodes, dedupedSelectedNodes: d.dedupedSelectedNodes, selectedBytes: d.selectedBytes,
    requiresConfirmation: d.requiresConfirmation, rootTotalBytes: d.rootTotalBytes, allowDeleteUnknown,
    runAnalyzer: api.runAnalyzer, toggleExpand: (node: AnalyzerNodeDto) => s.setExpanded((prev) => toggleExpandedId(prev, node)),
    toggleSelect: api.toggleSelect, selectSafeAnalyzer: api.selectSafeAnalyzer, runPreview: api.runPreview, runClean: api.runClean,
    focusNode: (input: { id: string }) => focusNodeInTree({ s, d, id: input.id, inspect: api.inspectNode }),
    cycleBrowsePath: () => s.setRootPath(nextBrowsePath({ current: s.rootPath })),
    toggleExpandCollapseAll: (ids: string[]) => toggleExpandAll({ s, ids }),
    inspectNode: (input: { id: string }) => api.inspectNode(input.id),
    setPreview: p.setPreview, setSelectedIds: s.setSelectedIds, setCleanSummary: c.setCleanSummary, setCleanProgress: c.setCleanProgress, invalidatePreviewRequest: api.invalidatePreviewRequest,
  };
}