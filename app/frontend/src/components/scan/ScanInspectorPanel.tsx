import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import type { ExecuteCleanResponse } from '../../api/client';
import { CleanSummaryBlock, PreviewBreakdownBlock, TYPED_CONFIRMATION_PHRASE, type ExplainState, type PreviewResult } from '../cleanShared';
import { ScanGroupInspector } from './ScanGroupInspector';
import { ScanItemInspector } from './ScanItemInspector';

interface PanelProps {
  focus: { kind: 'item'; itemId: string } | { kind: 'group'; targetId: string } | null;
  focusedGroup: TargetScanResultDto | null;
  inspected: ExplainState;
  inspectedCtx: { target: TargetScanResultDto; item: ScanItemDto } | null;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  requiresConfirmation: boolean;
  confirmText: string;
  onConfirmText: (v: string) => void;
  cleaning: boolean;
  preview: PreviewResult | null;
  cleanProgressLabel: string | null;
  cleanSummary: ExecuteCleanResponse | null;
  onOpenLog: (id: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function ConfirmationNotice(props: { show: boolean; confirmText: string; onConfirmText: (v: string) => void; cleaning: boolean }) {
  if (!props.show) return null;
  return (
    <div className="notice warn" role="note">
      <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path items). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
      <div className="field"><label htmlFor="confirm-delete">Typed confirmation</label><input id="confirm-delete" type="text" value={props.confirmText} onChange={(e) => props.onConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={props.cleaning} /></div>
    </div>
  );
}

function PreviewSection(props: { preview: PreviewResult | null; cleaning: boolean; label: string | null }) {
  if (!props.preview) return null;
  return <PreviewBreakdownBlock preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.label} />;
}

function SummarySection(props: { summary: ExecuteCleanResponse | null; onOpenLog: (id: string) => void }) {
  if (!props.summary) return null;
  return <CleanSummaryBlock summary={props.summary} onOpenLog={props.onOpenLog} />;
}

function showGroup(props: PanelProps): boolean {
  if (props.focus?.kind !== 'group') return false;
  return !!props.focusedGroup;
}

function showItem(props: PanelProps): boolean {
  if (props.focus?.kind !== 'item') return false;
  if (!props.inspected) return false;
  return !!props.inspectedCtx;
}

function InspectorBody(props: PanelProps) {
  if (showGroup(props)) return <ScanGroupInspector target={props.focusedGroup!} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} selectedIds={props.selectedIds} onOpenSettings={props.onOpenSettings} />;
  if (showItem(props)) return <ScanItemInspector target={props.inspectedCtx!.target} item={props.inspectedCtx!.item} inspected={props.inspected} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} onOpenSettings={props.onOpenSettings} />;
  return <p className="inspector-empty">Select an item to see its purpose, risk, full path, and eligibility.</p>;
}

export function ScanInspectorPanel(props: PanelProps) {
  return (
    <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="inspector-heading">
      <div className="pane-header"><h3 id="inspector-heading">Inspector</h3></div>
      <div className="detail-scroll">
        <div className="inspector-section" aria-label="Item inspector"><InspectorBody {...props} /></div>
        <ConfirmationNotice show={props.requiresConfirmation} confirmText={props.confirmText} onConfirmText={props.onConfirmText} cleaning={props.cleaning} />
        <PreviewSection preview={props.preview} cleaning={props.cleaning} label={props.cleanProgressLabel} />
        <SummarySection summary={props.cleanSummary} onOpenLog={props.onOpenLog} />
      </div>
    </aside>
  );
}
