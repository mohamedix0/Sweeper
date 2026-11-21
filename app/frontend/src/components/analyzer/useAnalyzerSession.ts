import { useRef, useState } from 'react';
import { formatContractError, type AnalyzerProgressEvent, type StartAnalyzerResult, type SweeperClient } from '../../api/client';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { type ExplainState } from '../cleanShared';

export function useAnalyzerSession(args: { client: SweeperClient }) {
  const [rootPath, setRootPath] = useState('C:\\');
  const [analyzedPath, setAnalyzedPath] = useState<string | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [progress, setProgress] = useState<AnalyzerProgressEvent | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [runningSessionId, setRunningSessionId] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [result, setResult] = useState<StartAnalyzerResult | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [showAllAnalyzerNodes, setShowAllAnalyzerNodes] = useState<Set<string>>(new Set());
  const inspectReqIdRef = useRef(0);

  function applyInspectResult(input: { reqId: number; result: ExplainState }) {
    if (inspectReqIdRef.current === input.reqId) setInspected(input.result);
  }

  async function inspectNode(input: { itemId: string }) {
    const reqId = inspectReqIdRef.current + 1;
    inspectReqIdRef.current = reqId;
    try {
      const result = await args.client.explainItem(input.itemId);
      applyInspectResult({ reqId, result });
    } catch { /* ignore */ }
  }

  function resetInspection() {
    inspectReqIdRef.current += 1;
    setInspected(null);
  }

  return {
    rootPath, setRootPath, analyzedPath, setAnalyzedPath, analyzing, setAnalyzing, progress, setProgress, sessionId, setSessionId, runningSessionId, setRunningSessionId, startedAt, setStartedAt,
    result, setResult, expanded, setExpanded, selectedIds, setSelectedIds, inspected, setInspected,
    showAllAnalyzerNodes, setShowAllAnalyzerNodes, inspectReqIdRef, inspectNode, resetInspection,
  };
}
