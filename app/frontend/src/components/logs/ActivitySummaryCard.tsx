import { CheckIcon } from '../Icons';

export function ActivitySummaryCard(props: {
  summary: { primaryLabel: string; primaryValue: string; eligible: string; skipped: string; outcome: string; mode: string | null };
}) {
  const isFound = props.summary.primaryLabel === 'Space Found';
  const complete = /completed/i.test(props.summary.outcome);
  return (
    <div className="activity-summary-strip activity-summary-cards" aria-label="Activity summary">
      <div className="activity-summary-cell">
        <span className="activity-summary-label">{isFound ? 'SPACE FOUND' : 'SPACE RECLAIMED'}</span>
        <span className="activity-summary-value num">{props.summary.primaryValue}</span>
      </div>
      <div className="activity-summary-cell">
        <span className="activity-summary-label">{isFound ? 'SAFE' : 'CLEANED'}</span>
        <span className="activity-summary-value num">{props.summary.eligible}</span>
      </div>
      <div className="activity-summary-cell">
        <span className="activity-summary-label">SKIPPED</span>
        <span className="activity-summary-value num">{props.summary.skipped}</span>
      </div>
      <div className="activity-summary-cell">
        <span className="activity-summary-label">OUTCOME</span>
        <span className={`activity-summary-value activity-summary-outcome ${complete ? 'is-complete' : ''}`}>
          {complete && <CheckIcon aria-hidden="true" />}
          <span>{props.summary.outcome}</span>
        </span>
      </div>
    </div>
  );
}
