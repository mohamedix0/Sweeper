import { CopyIcon, ExportIcon } from '../Icons';
import { ActivitySummaryCard } from './ActivitySummaryCard';
import { AuditTable } from './AuditTable';
import type { AuditRow, LogMeta, OutcomeFilter } from './logParsers';

type Summary = { title: string; subtitle: string; whenAt: string; kind?: string } | null;
type Activity = { primaryLabel: string; primaryValue: string; eligible: string; skipped: string; outcome: string; mode: string | null };

function DetailsActions(props: { mode: 'structured' | 'raw'; copied: boolean; showScanAgain: boolean; onMode: (m: 'structured' | 'raw') => void; onCopy: () => void; onExport: () => void; onScanAgain?: () => void }) {
  return (
    <div className="log-header-actions">
      <div className="segmented-toggle" role="group" aria-label="Log view mode">
        <button type="button" className={`seg-btn ${props.mode === 'structured' ? 'active' : ''}`} aria-pressed={props.mode === 'structured'} onClick={() => props.onMode('structured')}>Summary</button>
        <button type="button" className={`seg-btn ${props.mode === 'raw' ? 'active' : ''}`} aria-pressed={props.mode === 'raw'} aria-label="Raw JSONL" onClick={() => props.onMode('raw')}>Raw Log</button>
      </div>
      <button type="button" className="btn small subtle-btn" onClick={props.onCopy} aria-label={props.copied ? 'Copied' : 'Copy log'} title="Copy"><CopyIcon /><span>{props.copied ? 'Copied' : 'Copy'}</span></button>
      <button type="button" className="btn small subtle-btn" onClick={props.onExport} aria-label="Export log" title="Export"><ExportIcon /><span>Export</span></button>
      {props.showScanAgain && props.onScanAgain && <button type="button" className="btn small secondary-action" onClick={props.onScanAgain} aria-label="Scan Again" title="Scan Again"><span>Scan Again</span></button>}
    </div>
  );
}

export function LogsDetailsPane(props: {
  selected: { id: string; content: string } | null;
  meta: LogMeta | null;
  summary: Summary;
  activity: Activity;
  mode: 'structured' | 'raw';
  copied: boolean;
  auditRows: AuditRow[];
  visibleRows: AuditRow[];
  outcomeFilter: OutcomeFilter;
  expanded: Set<string>;
  hasLogs: boolean;
  hasFilteredLogs: boolean;
  onMode: (m: 'structured' | 'raw') => void;
  onCopy: () => void;
  onExport: () => void;
  onOutcome: (f: OutcomeFilter) => void;
  onToggle: (key: string) => void;
  onScanAgain?: () => void;
}) {
  const showScanAgain = props.summary?.kind === 'scan' && !!props.onScanAgain;
  const emptyText = !props.hasFilteredLogs && props.hasLogs ? 'No matching activity for this filter' : 'No activity selected';
  return (
    <div className="logs-pane logs-pane-right">
      <div className="logs-pane-header log-content-header">
        <h2 className="pane-section-title">Activity Details</h2><div className="spacer" />
        {props.selected && <DetailsActions mode={props.mode} copied={props.copied} showScanAgain={showScanAgain} onMode={props.onMode} onCopy={props.onCopy} onExport={props.onExport} onScanAgain={props.onScanAgain} />}
      </div>
      <div className="logs-pane-body log-details">
        {props.selected ? (
          <>
            <div aria-label="Structured log summary" className="log-summary-head">
              {props.summary && <div className="log-summary-title">{props.summary.title}</div>}
              {props.summary && <div className="muted log-summary-meta"><span>{props.summary.whenAt}{props.activity.mode ? <> · <span className="log-mode-badge">Mode: {props.activity.mode}</span></> : null} · {props.summary.subtitle}</span></div>}
            </div>
            <ActivitySummaryCard summary={props.activity} />
            {props.mode === 'structured' ? (
              <AuditTable auditRows={props.auditRows} visibleRows={props.visibleRows} outcomeFilter={props.outcomeFilter} onOutcome={props.onOutcome} expanded={props.expanded} onToggle={props.onToggle} primaryLabel={props.activity.primaryLabel} kind={props.summary?.kind} />
            ) : <pre className="log" aria-label="Log content">{props.selected.content}</pre>}
          </>
        ) : <p className="muted pad-md">{emptyText}</p>}
      </div>
    </div>
  );
}
