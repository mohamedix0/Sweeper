import type { AnalyzerProgressEvent, StartAnalyzerResult } from '../../api/client';

/// Everything that belongs to one Analyzer run. A stopped, cancelled, or
/// invalidated run must converge all of it so no stale path/session survives
/// into the next run (#99).
export type AnalyzerRunState = {
  setAnalyzing: (v: boolean) => void;
  setProgress: (v: AnalyzerProgressEvent | null) => void;
  setRunningSessionId: (v: string | null) => void;
  setSessionId: (v: string | null) => void;
  setStartedAt: (v: number | null) => void;
  setResult: (v: StartAnalyzerResult | null) => void;
  setAnalyzedPath: (v: string | null) => void;
  setSelectedIds: (v: Set<string>) => void;
  setExpanded: (v: Set<string>) => void;
  setShowAllAnalyzerNodes: (v: Set<string>) => void;
  resetInspection: () => void;
};

/// Single convergence path shared by Stop, the cancelled-result branch and
/// session reset. Keeping it in one place prevents the fields from drifting
/// apart between those callers.
export function convergeStop(s: AnalyzerRunState): void {
  s.setAnalyzing(false);
  s.setProgress(null);
  s.setRunningSessionId(null);
  s.setSessionId(null);
  s.setStartedAt(null);
  s.setResult(null);
  s.setAnalyzedPath(null);
  s.setSelectedIds(new Set());
  s.setExpanded(new Set());
  s.setShowAllAnalyzerNodes(new Set());
  s.resetInspection();
}
