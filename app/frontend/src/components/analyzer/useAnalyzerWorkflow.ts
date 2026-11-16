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
type SessionModel = ReturnType<typeof useAnalyzerSession>;
type DerivedModel = ReturnType<typeof useAnalyzerDerived>;
type PreviewCleanState = ReturnType<typeof usePreviewCleanState>;
type MessageState = ReturnType<typeof useRunMessages>;

function nextBrowsePath(current: string) {
  const idx = BROWSE_PATHS.findIndex((p: string) => p.toLowerCase() === current.trim().toLowerCase());
  return BROWSE_PATHS[(idx + 1 + BROWSE_PATHS.length) % BROWSE_PATHS.length];
}

function collectSafeSelectable(input: { nodesById: Map<string, AnalyzerNodeDto>; allowDeleteUnknown: boolean }) {
  const safeNodes: AnalyzerNodeDto[] = [];
  input.nodesById.forEach((node) => {
    if (!isNodeSelectable({ node, allowDeleteUnknown: input.allowDeleteUnknown })) return;
    if (analyzerStatusKey({ node }) !== 'safe') return;
    safeNodes.push(node);
  });
  return dedupeAnalyzerNodesByCoverage(safeNodes);
}

function usePreviewCleanState() {
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const rev = useRef(0);
  const active = useRef(0);
  return { preview, setPreview, previewLoading, setPreviewLoading, confirmText, setConfirmText, cleaning, setCleaning, cleanProgress, setCleanProgress, cleanSummary, setCleanSummary, rev, active };
}

function useRunMessages() {
  const [error, setError] = useState<string | null>(null);
  const [info, setInfoState] = useState<string | null>(null);
  return { error, setError, info, setInfo: setInfoState };
}

function createAnalyzerActions(input: { args: WfArgs; s: SessionModel; d: DerivedModel; m: MessageState; p: PreviewCleanState; allowDeleteUnknown: boolean; safeSelectableNodes: AnalyzerNodeDto[] }) {
  const { args, s, d, m, p, allowDeleteUnknown, safeSelectableNodes } = input;
  function invalidatePreviewRequest() { p.rev.current += 1; p.active.current += 1; p.setPreviewLoading(false); }
  function clearPreviewState() { p.setPreview(null); p.setCleanSummary(null); p.setCleanProgress(null); }
  function inspectNodeById(id: string) { void s.inspectNode({ itemId: id }); }
  function resetAnalyzerSession(msg?: string) {
    invalidatePreviewRequest();
    s.resetInspection(); s.setSessionId(null); s.setStartedAt(null); s.setResult(null); s.setSelectedIds(new Set()); s.setExpanded(new Set()); s.setShowAllAnalyzerNodes(new Set());
    p.setPreview(null); p.setConfirmText(''); p.setCleanProgress(null);
    if (msg) m.setInfo(msg);
  }
  async function runAnalyzer(run?: { path?: string }) {
    await runAnalyzerAction({ client: args.client, pathOverride: run?.path, rootPath: s.rootPath, setRootPath: s.setRootPath, invalidatePreviewRequest, resetInspection: s.resetInspection, clearRunState: () => { m.setError(null); m.setInfo(null); clearPreviewState(); }, setSelectedIds: s.setSelectedIds, setResult: s.setResult, setExpanded: s.setExpanded, setShowAllAnalyzerNodes: s.setShowAllAnalyzerNodes, setAnalyzing: s.setAnalyzing, setProgress: s.setProgress, setSessionId: s.setSessionId, setStartedAt: s.setStartedAt, setAnalyzedPath: s.setAnalyzedPath, inspectNode: ({ id }) => inspectNodeById(id), setError: m.setError });
  }
  function toggleSelect(node: AnalyzerNodeDto) {
    if (!isNodeSelectable({ node, allowDeleteUnknown })) { inspectNodeById(node.id); return; }
    invalidatePreviewRequest(); clearPreviewState();
    s.setSelectedIds((prev) => nextSelectionAfterToggle({ prev, node, nodesById: d.nodesById }));
    inspectNodeById(node.id);
  }
  function selectSafeAnalyzer() {
    invalidatePreviewRequest(); clearPreviewState();
    s.setSelectedIds((prev) => mergeSafeSelection({ prev, additions: safeSelectableNodes, nodesById: d.nodesById }));
  }
  async function runPreview() {
    await runPreviewAction({ client: args.client, sessionId: s.sessionId, dedupedSelectedNodes: d.dedupedSelectedNodes, requiresConfirmation: d.requiresConfirmation, confirmText: p.confirmText, refs: { previewRevision: p.rev, activePreviewRequest: p.active }, setError: m.setError, setInfo: m.setInfo, setPreviewLoading: p.setPreviewLoading, setPreview: p.setPreview, setCleanSummary: p.setCleanSummary, setCleanProgress: p.setCleanProgress, resetAnalyzerSession });
  }
  async function runClean() {
    await runCleanAction({ client: args.client, preview: p.preview, setError: m.setError, setInfo: m.setInfo, setCleaning: p.setCleaning, setCleanProgress: p.setCleanProgress, setCleanSummary: p.setCleanSummary, setPreview: p.setPreview, setSelectedIds: s.setSelectedIds, setConfirmText: p.setConfirmText, resetAnalyzerSession });
  }
  function focusNode(target: { id: string }) {
    const node = d.nodesById.get(target.id);
    if (!node) return;
    const result = s.result;
    if (result) s.setExpanded((prev) => new Set([...prev, ...expandAncestorsForNode({ root: result.root, nodeId: target.id })]));
    inspectNodeById(target.id);
  }
  function toggleExpandCollapseAll(ids: string[]) {
    if (!s.result) return;
    const allOpen = ids.length > 0 && ids.every((id) => s.expanded.has(id));
    if (allOpen) { s.setExpanded(new Set()); return; }
    s.setExpanded(new Set(ids));
  }
  return { invalidatePreviewRequest, inspectNodeById, resetAnalyzerSession, runAnalyzer, toggleSelect, selectSafeAnalyzer, runPreview, runClean, focusNode, toggleExpandCollapseAll };
}

export function useAnalyzerWorkflow(args: WfArgs) {
  const s = useAnalyzerSession({ client: args.client });
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'size' | 'items' | 'name'>('size');
  const [statusFilter, setStatusFilter] = useState<AnalyzerStatusFilter>('all');
  const m = useRunMessages();
  const p = usePreviewCleanState();
  const allowDeleteUnknown = args.settings?.allowDeleteUnknown ?? false;
  const d = useAnalyzerDerived({ result: s.result, query, sort, statusFilter, expanded: s.expanded, showAllAnalyzerNodes: s.showAllAnalyzerNodes, selectedIds: s.selectedIds });
  const safeSelectableNodes = useMemo(() => collectSafeSelectable({ nodesById: d.nodesById, allowDeleteUnknown }), [d.nodesById, allowDeleteUnknown]);
  const actions = createAnalyzerActions({ args, s, d, m, p, allowDeleteUnknown, safeSelectableNodes });

  return {
    rootPath: s.rootPath, setRootPath: s.setRootPath, analyzedPath: s.analyzedPath, analyzing: s.analyzing, progress: s.progress,
    sessionId: s.sessionId, result: s.result, expanded: s.expanded, selectedIds: s.selectedIds, inspected: s.inspected,
    preview: p.preview, previewLoading: p.previewLoading, confirmText: p.confirmText, setConfirmText: p.setConfirmText,
    cleaning: p.cleaning, cleanProgress: p.cleanProgress, cleanSummary: p.cleanSummary, error: m.error, info: m.info,
    query, setQuery, sort, setSort, statusFilter, setStatusFilter,
    showAllAnalyzerNodes: s.showAllAnalyzerNodes, setShowAllAnalyzerNodes: s.setShowAllAnalyzerNodes,
    nodesById: d.nodesById, filteredRoot: d.filteredRoot, flatRows: d.flatRows, statusCounts: d.statusCounts,
    selectedNodes: d.selectedNodes, dedupedSelectedNodes: d.dedupedSelectedNodes, selectedBytes: d.selectedBytes,
    requiresConfirmation: d.requiresConfirmation, rootTotalBytes: d.rootTotalBytes, allowDeleteUnknown,
    runAnalyzer: actions.runAnalyzer, toggleExpand: (node: AnalyzerNodeDto) => s.setExpanded((prev) => toggleExpandedId(prev, node)),
    toggleSelect: actions.toggleSelect, selectSafeAnalyzer: actions.selectSafeAnalyzer, runPreview: actions.runPreview, runClean: actions.runClean, focusNode: actions.focusNode,
    cycleBrowsePath: () => s.setRootPath(nextBrowsePath(s.rootPath)), toggleExpandCollapseAll: actions.toggleExpandCollapseAll,
    inspectNode: (input: { id: string }) => actions.inspectNodeById(input.id),
    setPreview: p.setPreview, setSelectedIds: s.setSelectedIds, setCleanSummary: p.setCleanSummary, setCleanProgress: p.setCleanProgress, invalidatePreviewRequest: actions.invalidatePreviewRequest,
  };
}