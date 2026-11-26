import { useEffect, useRef, useState } from 'react';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { normalizeDisplayPath } from '../../utils/format';
import { CleanSummaryBlock, ExplanationUnavailableNotice, InspectorMetaGrid, InspectorTitleRow, PathActions, PathWell, PreviewBreakdownBlock, TYPED_CONFIRMATION_PHRASE, copyText, type ExplainState, type PreviewResult } from '../cleanShared';
import { deriveCalloutItemsFromSelection, type PreviewRiskCalloutItem, type PreviewRiskCalloutSource } from '../PreviewRiskCallouts';
import { isCustomPathTargetId } from '../scan/scanHelpers';
import { CloudIcon, DriveIcon, FileIcon, FolderIcon, HardDriveIcon, LinkIcon, ShieldIcon } from '../Icons';
import { analyzerGuidance, analyzerStatusPill, analyzerStatusText, escapeVhdxPathForPowerShell, isDriveRoot, isNodeSelectable, isVhdxPath, nodeTypeTag } from './analyzerHelpers';
import type { ExecuteCleanResponse } from '../../api/client';

function fileOrFolderIcon(node: AnalyzerNodeDto) {
  if (node.isDir) return <FolderIcon className="inspector-leading-icon" />;
  return <FileIcon className="inspector-leading-icon" />;
}

function leadingInspectorIcon(node: AnalyzerNodeDto | null) {
  if (!node) return <FolderIcon className="inspector-leading-icon" />;
  if (isVhdxPath({ path: node.path })) return <HardDriveIcon className="inspector-leading-icon" />;
  if (node.isLinkPoint) return <LinkIcon className="inspector-leading-icon" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="inspector-leading-icon" />;
  if (node.risk === 'Protected') return <ShieldIcon className="inspector-leading-icon" />;
  if (isDriveRoot({ node })) return <DriveIcon className="inspector-leading-icon" />;
  return fileOrFolderIcon(node);
}

type AnalyzerInspectorProps = {
  inspected: ExplainState;
  inspectedNode: AnalyzerNodeDto | null;
  selectedIds: ReadonlySet<string>;
  selectionSources: readonly PreviewRiskCalloutSource[];
  allowDeleteUnknown: boolean;
  requiresConfirmation: boolean;
  confirmText: string;
  onConfirmText: (v: string) => void;
  preview: PreviewResult | null;
  cleaning: boolean;
  cleanProgressLabel: string | null;
  cleanSummary: ExecuteCleanResponse | null;
  onOpenLog: (id: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
};

function nodeSubtitle(node: AnalyzerNodeDto): string {
  const tag = nodeTypeTag({ node });
  if (tag) return tag;
  if (isDriveRoot({ node })) return 'Drive';
  if (node.isDir) return 'Folder';
  return 'File';
}

function badgeFor(pill: ReturnType<typeof analyzerStatusPill>, node: AnalyzerNodeDto) {
  if (pill) return <span className={pill.className}>{pill.label}</span>;
  return <span className="status-pill">{node.risk}</span>;
}

function eligibilityText(args: { node: AnalyzerNodeDto; selectable: boolean; status: ReturnType<typeof analyzerStatusText> | null }): string {
  if (isDriveRoot({ node: args.node })) return 'Drive root — expand C: below to inspect and select specific folders.';
  if (!args.status) return '—';
  if (args.selectable) return 'Meets age limit (> 24h old)';
  return args.status.text;
}

function VhdxCallout(props: { path: string; copied: boolean; onCopy: (cmd: string) => void }) {
  const cmd = `wsl --shutdown\nOptimize-VHD -Path ${escapeVhdxPathForPowerShell({ path: props.path })} -Mode Full`;
  let label = 'Copy Command';
  if (props.copied) label = 'Copied';
  return (
    <div className="notice info analyzer-vhdx-callout" role="note">
      <div><strong>Compaction (manual):</strong> shut down WSL/VM first, then run in an elevated PowerShell (Hyper-V).</div>
      <pre className="analyzer-vhdx-cmd"><code>{cmd}</code></pre>
      <button type="button" className="btn small subtle-btn analyzer-copy-cmd-btn" onClick={() => props.onCopy(cmd)} aria-label="Copy Command">{label}</button>
    </div>
  );
}

function UnknownSettingsLink(props: { show: boolean; onOpenSettings?: AnalyzerInspectorProps['onOpenSettings'] }) {
  if (!props.show) return null;
  if (!props.onOpenSettings) return null;
  return (
    <div className="inspector-policy-links" aria-label="Inspector actions">
      <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.('allowDeleteUnknown')}>Allow unknown folders in Settings →</button>
    </div>
  );
}

function ConfirmationBlock(props: { show: boolean; confirmText: string; onConfirmText: (v: string) => void; cleaning: boolean }) {
  if (!props.show) return null;
  return (
    <div className="notice warn" role="note">
      <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
      <div className="field"><label htmlFor="analyzer-confirm-delete">Typed confirmation</label><input id="analyzer-confirm-delete" type="text" value={props.confirmText} onChange={(e) => props.onConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={props.cleaning} /></div>
    </div>
  );
}

function inspectedTitle(props: AnalyzerInspectorProps & { node: AnalyzerNodeDto }): string {
  const title = props.inspected?.title;
  if (title) return title;
  return props.node.name;
}

function Guidance(props: { show: boolean; node: AnalyzerNodeDto; selectable: boolean; allowDeleteUnknown: boolean }) {
  if (!props.show) return null;
  return <p className="inspector-guidance">{analyzerGuidance({ node: props.node, selectable: props.selectable, allowDeleteUnknown: props.allowDeleteUnknown })}</p>;
}

function VhdxBlock(props: { show: boolean; path: string; copied: boolean; onCopy: (cmd: string) => void }) {
  if (!props.show) return null;
  return <VhdxCallout path={props.path} copied={props.copied} onCopy={props.onCopy} />;
}

function InspectorBody(props: AnalyzerInspectorProps & { node: AnalyzerNodeDto; selectable: boolean; status: ReturnType<typeof analyzerStatusText>; pill: ReturnType<typeof analyzerStatusPill>; copiedVhdxCmd: boolean; copyVhdxCommand: (cmd: string) => void }) {
  const { node } = props;
  const showUnknownLink = Boolean(props.status.unknownGated);
  let showGuidance = true;
  if (isDriveRoot({ node })) showGuidance = false;
  let showVhdx = false;
  if (isVhdxPath({ path: node.path })) showVhdx = true;
  const path = normalizeDisplayPath({ p: node.path });
  return (
    <>
      <InspectorTitleRow icon={leadingInspectorIcon(node)} title={inspectedTitle(props)} subtitle={nodeSubtitle(node)} badge={badgeFor(props.pill, node)} />
      {props.inspected?.failed ? <ExplanationUnavailableNotice /> : null}
      <InspectorMetaGrid size={node.sizeBytes} extraLabel="Items" extraValue={<span className="num">{node.itemCount}</span>} eligibility={eligibilityText({ node, selectable: props.selectable, status: props.status })} />
      <Guidance show={showGuidance} node={node} selectable={props.selectable} allowDeleteUnknown={props.allowDeleteUnknown} />
      <VhdxBlock show={showVhdx} path={node.path} copied={props.copiedVhdxCmd} onCopy={props.copyVhdxCommand} />
      <UnknownSettingsLink show={showUnknownLink} onOpenSettings={props.onOpenSettings} />
      <PathWell path={path} />
      <PathActions path={path} />
    </>
  );
}

function InspectorBodySlot(props: AnalyzerInspectorProps & { node: AnalyzerNodeDto | null; selectable: boolean; status: ReturnType<typeof analyzerStatusText> | null; pill: ReturnType<typeof analyzerStatusPill>; copiedVhdxCmd: boolean; copyVhdxCommand: (cmd: string) => void }) {
  if (!props.inspected) return <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>;
  if (props.node == null) return <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>;
  if (props.status == null) return <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>;
  return <InspectorBody {...props} node={props.node} selectable={props.selectable} status={props.status} pill={props.pill} copiedVhdxCmd={props.copiedVhdxCmd} copyVhdxCommand={props.copyVhdxCommand} />;
}

function PreviewSlot(props: { preview: PreviewResult | null; cleaning: boolean; cleanProgressLabel: string | null; calloutItems: PreviewRiskCalloutItem[] }) {
  if (!props.preview) return null;
  return <PreviewBreakdownBlock preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} calloutItems={props.calloutItems} />;
}

function SummarySlot(props: { summary: ExecuteCleanResponse | null; onOpenLog: (id: string) => void }) {
  if (!props.summary) return null;
  return <CleanSummaryBlock summary={props.summary} onOpenLog={props.onOpenLog} />;
}

export function AnalyzerInspectorPanel(props: AnalyzerInspectorProps) {
  const [copiedVhdxCmd, setCopiedVhdxCmd] = useState(false);
  const copyResetTimerRef = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
        copyResetTimerRef.current = null;
      }
    },
    [],
  );
  const node = props.inspectedNode;
  let selectable = false;
  let status: ReturnType<typeof analyzerStatusText> | null = null;
  let pill: ReturnType<typeof analyzerStatusPill> = null;
  if (node) {
    selectable = isNodeSelectable({ node, allowDeleteUnknown: props.allowDeleteUnknown });
    status = analyzerStatusText({ node, selectable, allowDeleteUnknown: props.allowDeleteUnknown });
    pill = analyzerStatusPill({ node, selectable });
  }
  function copyVhdxCommand(cmd: string) {
    void copyText({ text: cmd }).then(() => {
      setCopiedVhdxCmd(true);
      if (copyResetTimerRef.current !== null) window.clearTimeout(copyResetTimerRef.current);
      copyResetTimerRef.current = window.setTimeout(() => {
        setCopiedVhdxCmd(false);
        copyResetTimerRef.current = null;
      }, 1500);
    });
  }
  const calloutItems = deriveCalloutItemsFromSelection({
    selectedIds: props.selectedIds,
    sources: props.selectionSources,
    isCustomPath: (targetId) => isCustomPathTargetId({ targetId }),
  });
  return (
    <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="analyzer-preview-heading">
      <div className="pane-header"><h3 id="analyzer-preview-heading">Inspector</h3></div>
      <div className="detail-scroll">
        <div className="inspector-section" aria-label="Analyzer inspector">
          <InspectorBodySlot {...props} node={node} selectable={selectable} status={status} pill={pill} copiedVhdxCmd={copiedVhdxCmd} copyVhdxCommand={copyVhdxCommand} />
        </div>
        <ConfirmationBlock show={props.requiresConfirmation} confirmText={props.confirmText} onConfirmText={props.onConfirmText} cleaning={props.cleaning} />
        <PreviewSlot preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} calloutItems={calloutItems} />
        <SummarySlot summary={props.cleanSummary} onOpenLog={props.onOpenLog} />
      </div>
    </aside>
  );
}
