import { formatContractError, type CleanProgressEvent, type ExecuteCleanResponse, type SweeperClient } from '../../api/client';
import type { AnalyzerNodeDto, ContractError } from '../../contract/bindings';
import type { ExplanationFallback, PreviewResult } from '../cleanShared';
import { convergeStop } from './analyzerStop';

type RunAnalyzerArgs = {
  client: SweeperClient;
  pathOverride?: string;
  rootPath: string;
  setRootPath: (v: string) => void;
  invalidatePreviewRequest: () => void;
  resetInspection: () => void;
  clearRunState: () => void;
  setSelectedIds: (v: Set<string>) => void;
  setResult: (v: any) => void;
  setExpanded: (v: Set<string>) => void;
  setShowAllAnalyzerNodes: (v: Set<string>) => void;
  setAnalyzing: (v: boolean) => void;
  setProgress: (v: any) => void;
  setSessionId: (v: string | null) => void;
  setStartedAt: (v: number | null) => void;
  setAnalyzedPath: (v: string | null) => void;
  setRunningSessionId: (v: string | null) => void;
  inspectNode: (input: { id: string; known?: ExplanationFallback }) => void;
  setError: (v: string | null) => void;
  setInfo: (v: string | null) => void;
  isCurrent: () => boolean;
};

function targetPathOf(args: { pathOverride?: string; rootPath: string }): string {
  let raw = args.rootPath;
  if (args.pathOverride) raw = args.pathOverride;
  const trimmed = raw.trim();
  if (trimmed) return trimmed;
  return 'C:\\';
}

function nodeBytes(node: AnalyzerNodeDto): bigint {
  const raw = node.sizeBytes;
  if (raw) return BigInt(raw);
  return 0n;
}

function compareDesc(a: AnalyzerNodeDto, b: AnalyzerNodeDto): number {
  const sa = nodeBytes(a);
  const sb = nodeBytes(b);
  if (sa === sb) return 0;
  if (sa > sb) return -1;
  return 1;
}

function autoInspectId(root: AnalyzerNodeDto): string {
  const sorted = [...(root.children ?? [])].sort(compareDesc);
  const first = sorted[0];
  if (first) return first.id;
  return root.id;
}

/** Find a node in a freshly returned tree (the render-time map is stale here). */
function analyzerNodeById(root: AnalyzerNodeDto, id: string): AnalyzerNodeDto | null {
  if (root.id === id) return root;
  for (const child of root.children ?? []) {
    const hit = analyzerNodeById(child, id);
    if (hit) return hit;
  }
  return null;
}

export async function runAnalyzerAction(args: RunAnalyzerArgs): Promise<void> {
  const targetPath = targetPathOf({ pathOverride: args.pathOverride, rootPath: args.rootPath });
  args.setRootPath(targetPath);
  args.invalidatePreviewRequest();
  args.resetInspection();
  args.clearRunState();
  args.setSelectedIds(new Set());
  args.setResult(null);
  args.setSessionId(null);
  args.setRunningSessionId(null);
  args.setExpanded(new Set());
  args.setShowAllAnalyzerNodes(new Set());
  args.setAnalyzing(true);
  args.setProgress(null);
  try {
    const res = await args.client.startAnalyzer(targetPath, (p) => {
      if (!args.isCurrent()) return;
      args.setRunningSessionId(p.sessionId);
      args.setProgress(p);
    });
    // A Stop can converge the UI while startAnalyzer is still in flight. Its
    // late result must never repopulate results for a run that is no longer
    // current (#99).
    if (!args.isCurrent()) return;
    if (res.cancelled) {
      convergeStop(args);
      args.setInfo('Analysis stopped.');
      return;
    }
    args.setResult(res);
    args.setSessionId(res.sessionId);
    args.setStartedAt(Date.now());
    args.setExpanded(new Set());
    args.setAnalyzedPath(res.rootPath || targetPath);
    // Use the fresh result's node for the known identity: the caller's
    // render-time `d.nodesById` is still empty on the first analysis (b1).
    const autoId = autoInspectId(res.root);
    const autoNode = analyzerNodeById(res.root, autoId);
    args.inspectNode({
      id: autoId,
      known: autoNode ? { title: autoNode.name, risk: autoNode.risk } : undefined,
    });
  } catch (e) {
    if (args.isCurrent()) args.setError(formatContractError(e));
  } finally {
    if (args.isCurrent()) {
      args.setAnalyzing(false);
      args.setRunningSessionId(null);
    }
  }
}
