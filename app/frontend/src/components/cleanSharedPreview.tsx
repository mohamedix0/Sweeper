import type { ExecuteCleanResponse } from '../api/client';
import { formatBytes, pluralize } from '../utils/format';
import type { PreviewResult } from './cleanSharedCore';
import { PreviewRiskCallouts, type PreviewRiskCalloutItem } from './PreviewRiskCallouts';

function previewTotalCount(args: { value: string }): number {
  return Number(args.value || '0');
}

function PreviewTable(props: { breakdown: PreviewResult['breakdown'] }) {
  return (
    <table className="table" aria-label="Preview breakdown by Target">
      <thead><tr><th>Target</th><th>Count</th><th>Size</th></tr></thead>
      <tbody>{props.breakdown.map((b) => <tr key={b.targetId}><td>{b.targetName}</td><td>{b.selectedCount}</td><td>{formatBytes({ bytes: b.selectedBytes })}</td></tr>)}</tbody>
    </table>
  );
}

function SkippedCountLine(props: { count: PreviewResult['skippedCount'] }) {
  if (props.count == null) return null;
  return <p className="muted small-note"><strong>Skipped:</strong> <span className="num">{props.count}</span></p>;
}

function SkippedReasonsList(props: { reasons: string[] }) {
  if (props.reasons.length === 0) return null;
  return (
    <ul className="skipped-reasons">
      {props.reasons.map((reason, idx) => <li key={`${reason}-${idx}`}>{reason}</li>)}
    </ul>
  );
}

function PreviewSkippedNotice(props: Pick<PreviewResult, 'skippedCount' | 'skippedReasons'>) {
  const { skippedCount, skippedReasons } = props;
  const reasons = skippedReasons ?? [];
  if (skippedCount == null) {
    if (reasons.length === 0) return null;
  }
  return (
    <div className="preview-skipped" role="status" aria-label="Preview skipped items">
      <strong className="subheading-xs">Skipped during preview</strong>
      <SkippedCountLine count={skippedCount} />
      <SkippedReasonsList reasons={reasons} />
    </div>
  );
}

function CleaningLabel(props: { label: string | null }) {
  if (!props.label) return null;
  return <div>{props.label}</div>;
}

function CleaningNotice(props: { label: string | null }) {
  return (
    <div className="notice info" role="status" aria-live="polite">
      <div>Cleaning in progress…</div>
      <CleaningLabel label={props.label} />
      <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
    </div>
  );
}

function CleaningBlock(props: { cleaning: boolean; label: string | null }) {
  if (!props.cleaning) return null;
  return <CleaningNotice label={props.label} />;
}

export function PreviewBreakdownBlock(props: { preview: PreviewResult; cleaning: boolean; cleanProgressLabel: string | null; calloutItems?: PreviewRiskCalloutItem[] }) {
  const count = previewTotalCount({ value: props.preview.totalSelectedCount });
  return (
    <div aria-live="polite" className="preview-breakdown">
      <h4 className="preview-breakdown-title">Preview Breakdown</h4>
      <p className="preview-total"><strong>Total selected:</strong> {pluralize({ count, singular: 'item', plural: 'items' })}, <span className="num">{formatBytes({ bytes: props.preview.totalSelectedBytes })}</span></p>
      <PreviewRiskCallouts items={props.calloutItems ?? []} />
      <PreviewTable breakdown={props.preview.breakdown} />
      <PreviewSkippedNotice skippedCount={props.preview.skippedCount} skippedReasons={props.preview.skippedReasons} />
      <div className="clean-footer" role="alert" aria-live="polite"><div className="clean-footer-warning"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div></div>
      <CleaningBlock cleaning={props.cleaning} label={props.cleanProgressLabel} />
    </div>
  );
}

function AbortedNotice(props: { summary: ExecuteCleanResponse }) {
  if (!props.summary.aborted) return null;
  let reason = props.summary.abortReason;
  if (!reason) reason = 'The operation was aborted.';
  return <div className="notice error" role="alert"><strong>Clean aborted.</strong> {reason}</div>;
}

function JournalWarning(props: { errors: string[] | undefined }) {
  const errors = props.errors ?? [];
  if (errors.length === 0) return null;
  return (
    <div className="notice warn" role="alert" aria-label="Journal warnings">
      <strong>Operation log warning.</strong> {errors.length} internal log write failure{errors.length === 1 ? '' : 's'} occurred after deletion. The results above reflect what actually happened, but the operation log may be incomplete; verify affected files on disk if needed.
    </div>
  );
}

function SkippedReasons(props: { reasons: ExecuteCleanResponse['skippedReasons'] }) {
  if (props.reasons.length === 0) return <p className="muted small-note" aria-label="Skipped reasons">None.</p>;
  return <ul aria-label="Skipped reasons" className="skipped-reasons">{props.reasons.map((r, idx) => <li key={`${r.reason}-${idx}`}>{r.reason} — {r.count}</li>)}</ul>;
}

function LogButton(props: { logId: string | null | undefined; onOpenLog: (logId: string) => void }) {
  if (!props.logId) return null;
  return <div className="row row-top"><button className="btn" onClick={() => props.onOpenLog(props.logId!)}>View Operation Log</button></div>;
}

export function CleanSummaryBlock(props: { summary: ExecuteCleanResponse; onOpenLog: (logId: string) => void }) {
  return (
    <div className="clean-summary-section" aria-labelledby="clean-summary-heading" role="status">
      <h4 id="clean-summary-heading" className="clean-summary-title">Clean Summary</h4>
      <AbortedNotice summary={props.summary} />
      <JournalWarning errors={props.summary.journalErrors} />
      <dl className="kv">
        <dt>Bytes freed</dt><dd className="num">{formatBytes({ bytes: props.summary.bytesFreed })}</dd>
        <dt>Deleted count</dt><dd className="num">{props.summary.deletedCount}</dd>
        <dt>Skipped count</dt><dd className="num">{props.summary.skippedCount}</dd>
      </dl>
      <div><strong className="subheading-xs">Skipped reasons</strong><SkippedReasons reasons={props.summary.skippedReasons} /></div>
      <LogButton logId={props.summary.logId} onOpenLog={props.onOpenLog} />
    </div>
  );
}
