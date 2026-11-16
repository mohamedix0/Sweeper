import { useState } from 'react';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { normalizeDisplayPath } from '../../utils/format';
import { CleanSummaryBlock, InspectorMetaGrid, InspectorTitleRow, PathActions, PathWell, PreviewBreakdownBlock, TYPED_CONFIRMATION_PHRASE, copyText, type ExplainState, type PreviewResult } from '../cleanShared';
import { CloudIcon, DriveIcon, FileIcon, FolderIcon, HardDriveIcon, LinkIcon, ShieldIcon } from '../Icons';
import { analyzerGuidance, analyzerStatusPill, analyzerStatusText, escapeVhdxPathForPowerShell, isDriveRoot, isNodeSelectable, isVhdxPath, nodeTypeTag } from './analyzerHelpers';
import type { ExecuteCleanResponse } from '../../api/client';

function leadingInspectorIcon(node: AnalyzerNodeDto | null) {
  if (!node) return <FolderIcon className="inspector-leading-icon" />;
  if (isVhdxPath(node.path)) return <HardDriveIcon className="inspector-leading-icon" />;
  if (node.isLinkPoint) return <LinkIcon className="inspector-leading-icon" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="inspector-leading-icon" />;
  if (node.risk === 'Protected') return <ShieldIcon className="inspector-leading-icon" />;
  if (isDriveRoot(node)) return <DriveIcon className="inspector-leading-icon" />;
  return node.isDir ? <FolderIcon className="inspector-leading-icon" /> : <FileIcon className="inspector-leading-icon" />;
}

export function AnalyzerInspectorPanel(props: {
  inspected: ExplainState;
  inspectedNode: AnalyzerNodeDto | null;
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
}) {
  const [copiedVhdxCmd, setCopiedVhdxCmd] = useState(false);
  const node = props.inspectedNode;
  const selectable = node ? isNodeSelectable(node, props.allowDeleteUnknown) : false;
  const status = node ? analyzerStatusText(node, selectable, props.allowDeleteUnknown) : null;
  const pill = node ? analyzerStatusPill(node, selectable, props.allowDeleteUnknown) : null;
  async function copyVhdxCommand(cmd: string) {
    await copyText(cmd);
    setCopiedVhdxCmd(true);
    window.setTimeout(() => setCopiedVhdxCmd(false), 1500);
  }
  return (
    <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="analyzer-preview-heading">
      <div className="pane-header"><h3 id="analyzer-preview-heading">Inspector</h3></div>
      <div className="detail-scroll">
        <div className="inspector-section" aria-label="Analyzer inspector">
          {props.inspected && node ? (
            <>
              <InspectorTitleRow icon={leadingInspectorIcon(node)} title={props.inspected.title || node.name} subtitle={nodeTypeTag(node) ?? (isDriveRoot(node) ? 'Drive' : 'Folder')} badge={pill ? <span className={pill.className}>{pill.label}</span> : <span className="status-pill">{node.risk}</span>} />
              <InspectorMetaGrid size={node.sizeBytes} extraLabel="Items" extraValue={<span className="num">{node.itemCount}</span>} eligibility={isDriveRoot(node) ? 'Drive root — expand C: below to inspect and select specific folders.' : (status ? (selectable ? 'Meets age limit (> 24h old)' : status.text) : '—')} />
              {!isDriveRoot(node) && <p className="inspector-guidance">{analyzerGuidance(node, selectable, props.allowDeleteUnknown)}</p>}
              {isVhdxPath(node.path) && (() => {
                const cmd = `wsl --shutdown\nOptimize-VHD -Path ${escapeVhdxPathForPowerShell(node.path)} -Mode Full`;
                return (
                  <div className="notice info analyzer-vhdx-callout" role="note">
                    <div><strong>Compaction (manual):</strong> shut down WSL/VM first, then run in an elevated PowerShell (Hyper-V).</div>
                    <pre className="analyzer-vhdx-cmd"><code>{cmd}</code></pre>
                    <button type="button" className="btn small subtle-btn analyzer-copy-cmd-btn" onClick={() => void copyVhdxCommand(cmd)} aria-label="Copy Command">{copiedVhdxCmd ? 'Copied' : 'Copy Command'}</button>
                  </div>
                );
              })()}
              {status?.unknownGated && props.onOpenSettings && (
                <div className="inspector-policy-links" aria-label="Inspector actions">
                  <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.('allowDeleteUnknown')}>Allow unknown folders in Settings →</button>
                </div>
              )}
              <PathWell path={normalizeDisplayPath(node.path)} />
              <PathActions path={normalizeDisplayPath(node.path)} />
            </>
          ) : <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>}
        </div>
        {props.requiresConfirmation && (
          <div className="notice warn" role="note">
            <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
            <div className="field"><label htmlFor="analyzer-confirm-delete">Typed confirmation</label><input id="analyzer-confirm-delete" type="text" value={props.confirmText} onChange={(e) => props.onConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={props.cleaning} /></div>
          </div>
        )}
        {props.preview && <PreviewBreakdownBlock preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} />}
        {props.cleanSummary && <CleanSummaryBlock summary={props.cleanSummary} onOpenLog={props.onOpenLog} />}
      </div>
    </aside>
  );
}
