import { useMemo } from 'react';
import { AllocBar } from './analyzer/analyzerAlloc';
import { AnalyzerResults } from './analyzer/AnalyzerResults';
import { AnalyzerHeader, AnalyzerLocationBar, AnalyzerProgressNotice } from './analyzer/AnalyzerViewParts';
import { buildAnalyzerResultsProps } from './analyzer/analyzerResultsProps';
import { useAnalyzerViewModel } from './analyzer/useAnalyzerViewModel';
import { useAnalyzerWorkflow } from './analyzer/useAnalyzerWorkflow';
import type { Settings, SweeperClient } from '../api/client';
import type { PreviewRiskCalloutSource } from './PreviewRiskCallouts';

type ViewProps = { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void; onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void };

function showAllocOf(input: { hasResult: boolean; segmentCount: number }): boolean {
  if (!input.hasResult) return false;
  return input.segmentCount > 0;
}

function maybeAllocBar(input: { showAlloc: boolean; w: ReturnType<typeof useAnalyzerWorkflow>; vm: ReturnType<typeof useAnalyzerViewModel> }) {
  if (!input.showAlloc) return null;
  if (!input.w.result) return null;
  return <AllocBar segments={input.vm.allocSegments} result={input.w.result} displayPath={input.vm.displayPath} analyzing={input.w.analyzing} cleaning={input.w.cleaning} onRun={(args) => void input.w.runAnalyzer({ path: args.path })} onFocus={(args) => input.w.focusNode(args)} />;
}

function maybeProgress(input: { analyzing: boolean; scannedDirs?: string; scannedFiles?: string; scannedBytes?: string; currentPath?: string | null; onStop: () => void }) {
  if (!input.analyzing) return null;
  return <AnalyzerProgressNotice scannedDirs={input.scannedDirs} scannedFiles={input.scannedFiles} scannedBytes={input.scannedBytes} currentPath={input.currentPath} onStop={input.onStop} />;
}

function maybeResults(input: { hasResult: boolean; props: ReturnType<typeof buildAnalyzerResultsProps>; selectionSources: readonly PreviewRiskCalloutSource[] }) {
  if (!input.hasResult) return null;
  return <AnalyzerResults {...input.props} selectionSources={input.selectionSources} />;
}

function maybeNotice(input: { text: string | null; kind: 'error' | 'info' }) {
  if (!input.text) return null;
  let role: 'alert' | 'status' = 'status';
  if (input.kind === 'error') role = 'alert';
  return <div className={`notice ${input.kind}`} role={role}>{input.text}</div>;
}

export function AnalyzerView(props: ViewProps) {
  const { client, settings, onOpenLog, onOpenSettings } = props;
  const w = useAnalyzerWorkflow({ client, settings });
  const vm = useAnalyzerViewModel({ w });
  const canAnalyze = Boolean(w.rootPath.trim());
  const entryCount = (w.result?.root.children ?? []).length;
  const handlers = {
    onInspect: (args: { id: string }) => void w.inspectNode(args),
    onShowMore: (args: { parentId: string }) => w.setShowAllAnalyzerNodes((prev: Set<string>) => new Set([...prev, args.parentId])),
    onDrill: (args: { path: string }) => { w.setRootPath(args.path); void w.runAnalyzer({ path: args.path }); },
  };
  const resultsProps = buildAnalyzerResultsProps({ w, vm, onOpenLog, onOpenSettings, handlers });
  const selectionSources = useMemo<readonly PreviewRiskCalloutSource[]>(
    () => w.selectedNodes.map((n) => ({
      itemId: n.id,
      title: n.name || n.path,
      description: n.description,
      risk: n.risk,
      targetId: n.targetId,
    })),
    [w.selectedNodes],
  );
  const showAlloc = showAllocOf({ hasResult: Boolean(w.result), segmentCount: vm.allocSegments.length });
  return (
    <div className="view-stack view-stack-fill">
      <AnalyzerHeader hasResult={vm.hasResult} displayPath={vm.displayPath} totalBytes={w.result?.totalBytes} entryCount={entryCount} totalItems={w.result?.totalItems} analyzing={w.analyzing} cleaning={w.cleaning} canAnalyze={canAnalyze} onRun={() => void w.runAnalyzer()} onStop={() => void w.stopAnalyzer()} />
      {maybeNotice({ text: w.error, kind: 'error' })}
      {maybeNotice({ text: w.info, kind: 'info' })}
      <AnalyzerLocationBar rootPath={w.rootPath} hasResult={vm.hasResult} analyzing={w.analyzing} cleaning={w.cleaning} onRootPath={w.setRootPath} onRun={(path) => void w.runAnalyzer({ path })} onCycleBrowse={w.cycleBrowsePath} />
      {maybeAllocBar({ showAlloc, w, vm })}
      {maybeProgress({ analyzing: w.analyzing, scannedDirs: w.progress?.scannedDirs, scannedFiles: w.progress?.scannedFiles, scannedBytes: w.progress?.scannedBytes, currentPath: vm.progressPath, onStop: () => void w.stopAnalyzer() })}
      {maybeResults({ hasResult: Boolean(w.result), props: resultsProps, selectionSources })}
    </div>
  );
}
