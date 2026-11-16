import { useMemo, useRef, useState } from 'react';
import { formatContractError, type AnalyzerProgressEvent, type CleanProgressEvent, type ExecuteCleanResponse, type Settings, type StartAnalyzerResult, type SweeperClient } from '../../api/client';
import type { ContractError, AnalyzerNodeDto } from '../../contract/bindings';
import { TYPED_CONFIRMATION_PHRASE, type ExplainState, type PreviewResult } from '../cleanShared';
import { analyzerSelectionNeedsConfirmation, analyzerStatusKey, buildFlat, BROWSE_PATHS, dedupeAnalyzerNodesByCoverage, filterTree, isAncestorAnalyzerPath, isNodeSelectable, type AnalyzerStatusFilter } from './analyzerHelpers';

export function useAnalyzerWorkflow(args: { client: SweeperClient; settings: Settings | null }) {
  const { client, settings } = args;
  const [rootPath, setRootPath] = useState('C:\\');
  const [analyzedPath, setAnalyzedPath] = useState<string | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [progress, setProgress] = useState<AnalyzerProgressEvent | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [, setStartedAt] = useState<number | null>(null);
  const [result, setResult] = useState<StartAnalyzerResult | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'size' | 'items' | 'name'>('size');
  const [statusFilter, setStatusFilter] = useState<AnalyzerStatusFilter>('all');
  const [showAllAnalyzerNodes, setShowAllAnalyzerNodes] = useState<Set<string>>(new Set());
  const previewRevisionRef = useRef(0);
  const activePreviewRequestRef = useRef(0);
  const inspectReqIdRef = useRef(0);
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  const invalidatePreviewRequest = () => {
    previewRevisionRef.current += 1;
    activePreviewRequestRef.current += 1;
    setPreviewLoading(false);
  };

  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    const walk = (n: AnalyzerNodeDto) => {
      map.set(n.id, n);
      for (const c of n.children ?? []) walk(c);
    };
    if (result?.root) walk(result.root);
    return map;
  }, [result]);

  const filteredRoot = useMemo(() => (
    result ? filterTree(result.root, query, sort, statusFilter) : null
  ), [result, query, sort, statusFilter]);

  const flatRows = useMemo(() => (
    filteredRoot ? buildFlat(filteredRoot, expanded, showAllAnalyzerNodes) : []
  ), [filteredRoot, expanded, showAllAnalyzerNodes]);

  const statusCounts = useMemo(() => {
    const counts = { all: 0, safe: 0, caution: 0, unknown: 0, locked: 0 };
    if (!result) return counts;
    const kids = result.root.children ?? [];
    counts.all = kids.length;
    for (const n of kids) {
      const k = analyzerStatusKey(n);
      if (k === 'safe') counts.safe += 1;
      else if (k === 'caution') counts.caution += 1;
      else if (k === 'unknown') counts.unknown += 1;
      else counts.locked += 1;
    }
    return counts;
  }, [result]);

  const selectedNodes = useMemo(() => (
    Array.from(selectedIds).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[]
  ), [selectedIds, nodesById]);

  const dedupedSelectedNodes = useMemo(() => dedupeAnalyzerNodesByCoverage(selectedNodes), [selectedNodes]);
  const selectedBytes = useMemo(() => (
    dedupedSelectedNodes.reduce((a, n) => a + BigInt(n.sizeBytes || '0'), 0n).toString()
  ), [dedupedSelectedNodes]);
  const requiresConfirmation = useMemo(() => analyzerSelectionNeedsConfirmation(dedupedSelectedNodes), [dedupedSelectedNodes]);
  const rootTotalBytes = useMemo(() => BigInt(result?.root.sizeBytes || result?.totalBytes || '0') || 1n, [result]);

  function resetAnalyzerSession(msg?: string) {
    invalidatePreviewRequest();
    inspectReqIdRef.current += 1;
    setPreviewLoading(false);
    setSessionId(null);
    setStartedAt(null);
    setResult(null);
    setSelectedIds(new Set());
    setExpanded(new Set());
    setShowAllAnalyzerNodes(new Set());
    setPreview(null);
    setConfirmText('');
    setCleanProgress(null);
    setInspected(null);
    if (msg) setInfo(msg);
  }

  async function inspectNode(itemId: string) {
    const reqId = inspectReqIdRef.current + 1;
    inspectReqIdRef.current = reqId;
    try {
      const r = await client.explainItem(itemId);
      if (inspectReqIdRef.current === reqId) setInspected(r);
    } catch { /* ignore */ }
  }

  async function runAnalyzer(pathOverride?: string) {
    const targetPath = (pathOverride ?? rootPath).trim() || 'C:\\';
    setRootPath(targetPath);
    invalidatePreviewRequest();
    inspectReqIdRef.current += 1;
    setPreviewLoading(false);
    setError(null);
    setInfo(null);
    setPreview(null);
    setCleanSummary(null);
    setCleanProgress(null);
    setSelectedIds(new Set());
    setResult(null);
    setExpanded(new Set());
    setShowAllAnalyzerNodes(new Set());
    setAnalyzing(true);
    setProgress(null);
    setInspected(null);
    try {
      const res = await client.startAnalyzer(targetPath, (p) => setProgress(p));
      setResult(res);
      setSessionId(res.sessionId);
      setStartedAt(Date.now());
      setExpanded(new Set());
      setAnalyzedPath(res.rootPath || targetPath);
      const autoInspectChild = [...(res.root.children ?? [])].sort((a, b) => {
        const sa = BigInt(a.sizeBytes || '0');
        const sb = BigInt(b.sizeBytes || '0');
        return sa === sb ? 0 : (sa > sb ? -1 : 1);
      })[0];
      void inspectNode((autoInspectChild ?? res.root).id);
      setInfo(null);
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setAnalyzing(false);
    }
  }

  function toggleExpand(node: AnalyzerNodeDto) {
    if (!node.isDir || !node.children || node.children.length === 0) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(node.id)) next.delete(node.id);
      else next.add(node.id);
      return next;
    });
  }

  function toggleSelect(node: AnalyzerNodeDto) {
    if (!isNodeSelectable(node, allowDeleteUnknown)) {
      void inspectNode(node.id);
      return;
    }
    invalidatePreviewRequest();
    setPreview(null);
    setCleanSummary(null);
    setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(node.id)) { next.delete(node.id); return next; }
      for (const id of Array.from(next)) {
        const existing = nodesById.get(id);
        if (existing && isAncestorAnalyzerPath(node.path, existing.path)) next.delete(id);
      }
      const hasSelectedAncestor = Array.from(next).some((id) => {
        const existing = nodesById.get(id);
        return !!existing && isAncestorAnalyzerPath(existing.path, node.path);
      });
      if (!hasSelectedAncestor) next.add(node.id);
      return next;
    });
    void inspectNode(node.id);
  }

  const safeSelectableNodeIds = useMemo(() => {
    const safeNodes: AnalyzerNodeDto[] = [];
    nodesById.forEach((node) => {
      if (!isNodeSelectable(node, allowDeleteUnknown)) return;
      const key = analyzerStatusKey(node);
      if (key === 'safe') safeNodes.push(node);
    });
    return dedupeAnalyzerNodesByCoverage(safeNodes).map((n) => n.id);
  }, [nodesById, allowDeleteUnknown]);

  function selectSafeAnalyzer() {
    invalidatePreviewRequest();
    setPreview(null);
    setCleanSummary(null);
    setCleanProgress(null);
    setSelectedIds((prev) => {
      const currentNodes = Array.from(prev).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
      const additions = safeSelectableNodeIds.map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
      const merged = [...currentNodes];
      for (const node of additions) {
        if (merged.some((m) => m.id === node.id || isAncestorAnalyzerPath(m.path, node.path))) continue;
        for (let i = merged.length - 1; i >= 0; i--) {
          if (isAncestorAnalyzerPath(node.path, merged[i].path)) merged.splice(i, 1);
        }
        merged.push(node);
      }
      return new Set(merged.map((n) => n.id));
    });
  }

  async function runPreview() {
    if (!sessionId) return;
    const requestId = activePreviewRequestRef.current + 1;
    activePreviewRequestRef.current = requestId;
    const requestRevision = previewRevisionRef.current;
    const requestSessionId = sessionId;
    const selectedForPreview = dedupeAnalyzerNodesByCoverage(selectedNodes);
    setError(null);
    setInfo(null);
    setPreviewLoading(true);
    setCleanSummary(null);
    setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(requestSessionId, selectedForPreview.map((n) => n.id), confirmations);
      if (activePreviewRequestRef.current !== requestId || previewRevisionRef.current !== requestRevision || sessionId !== requestSessionId) return;
      setPreview({ ...res, issuedAtMs: Date.now() });
    } catch (e) {
      if (activePreviewRequestRef.current !== requestId || previewRevisionRef.current !== requestRevision || sessionId !== requestSessionId) return;
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') setPreview(null);
    } finally {
      if (activePreviewRequestRef.current === requestId) setPreviewLoading(false);
    }
  }

  async function runClean() {
    if (!preview) return;
    setError(null);
    setInfo(null);
    setCleaning(true);
    setCleanProgress(null);
    setCleanSummary(null);
    try {
      const res = await client.executeClean(preview.previewToken, (ev) => setCleanProgress(ev));
      setCleanSummary(res);
      setPreview(null);
      setSelectedIds(new Set());
      setConfirmText('');
      setInfo('Clean completed. Review the summary and Activity History below.');
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') {
        setPreview(null);
        setInfo('Preview token is invalid, expired, or already used. Run Review & Clean again.');
      }
    } finally {
      setCleaning(false);
    }
  }

  function focusNode(nodeId: string) {
    const node = nodesById.get(nodeId);
    if (!node) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      const walk = (n: AnalyzerNodeDto, ancestors: string[]): boolean => {
        if (n.id === nodeId) { ancestors.forEach((a) => next.add(a)); return true; }
        for (const c of n.children ?? []) if (walk(c, [...ancestors, n.id])) return true;
        return false;
      };
      if (result) walk(result.root, []);
      return next;
    });
    void inspectNode(nodeId);
  }

  function cycleBrowsePath() {
    const idx = BROWSE_PATHS.findIndex((p: string) => p.toLowerCase() === rootPath.trim().toLowerCase());
    setRootPath(BROWSE_PATHS[(idx + 1 + BROWSE_PATHS.length) % BROWSE_PATHS.length]);
  }

  function toggleExpandCollapseAll(visibleExpandableIds: string[]) {
    if (!result) return;
    const allVisibleExpanded = visibleExpandableIds.length > 0 && visibleExpandableIds.every((id) => expanded.has(id));
    setExpanded(allVisibleExpanded ? new Set() : new Set(visibleExpandableIds));
  }

  return {
    rootPath, setRootPath, analyzedPath, analyzing, progress, sessionId, result, expanded, selectedIds, inspected, preview, previewLoading, confirmText, setConfirmText, cleaning, cleanProgress, cleanSummary, error, info, query, setQuery, sort, setSort, statusFilter, setStatusFilter, showAllAnalyzerNodes, setShowAllAnalyzerNodes,
    nodesById, filteredRoot, flatRows, statusCounts, selectedNodes, dedupedSelectedNodes, selectedBytes, requiresConfirmation, rootTotalBytes, allowDeleteUnknown,
    runAnalyzer, toggleExpand, toggleSelect, selectSafeAnalyzer, runPreview, runClean, focusNode, cycleBrowsePath, toggleExpandCollapseAll, inspectNode,
    setPreview, setSelectedIds, setCleanSummary, setCleanProgress, invalidatePreviewRequest,
  };
}
