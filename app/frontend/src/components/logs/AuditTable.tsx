import { Fragment } from 'react';
import type { AuditRow, OutcomeFilter } from './logParsers';
import { formatGroupBytes, groupSkippedRows } from './useLogsSelection';

function AuditRowView({ row, indented }: { row: AuditRow; indented: boolean }) {
  const unknown = row.outcome.toLowerCase().includes('unknown') || row.outcome.toLowerCase().includes('unverified');
  return (
    <tr className={`${row.kind === 'Category' ? 'audit-row-category' : 'audit-row-file'} ${indented ? 'audit-row-indented' : ''}`}>
      <td className="audit-category"><div className={`audit-category-cell ${indented ? 'audit-category-cell-indented' : ''}`}><span className="audit-category-main">{row.category}</span>{row.sublabel ? <span className="audit-kind muted">{row.sublabel}</span> : null}</div></td>
      <td className="audit-location mono" title={row.location}>{String(row.location || '')}</td>
      <td className="audit-size num">{row.size}</td>
      <td className="audit-outcome"><span className={`badge outcome-pill outcome-${row.outcomeTone}${unknown ? ' badge-unknown' : ''}`}>{row.outcome}</span></td>
    </tr>
  );
}

function OutcomeFilters(props: { filter: OutcomeFilter; onChange: (f: OutcomeFilter) => void }) {
  return (
    <div className="logs-outcome-filter" role="group" aria-label="Outcome filters">
      <div className="logs-outcome-inline-filters" role="group" aria-label="Outcome filters">
        <button type="button" className={`logs-outcome-link ${props.filter === 'all' ? 'active' : ''}`} aria-pressed={props.filter === 'all'} onClick={() => props.onChange('all')}>All</button>
        <button type="button" className={`logs-outcome-link ${props.filter === 'eligible' ? 'active' : ''}`} aria-pressed={props.filter === 'eligible'} onClick={() => props.onChange('eligible')}>Safe</button>
        <button type="button" className={`logs-outcome-link ${props.filter === 'skipped' ? 'active' : ''}`} aria-pressed={props.filter === 'skipped'} onClick={() => props.onChange('skipped')}>Skipped</button>
      </div>
    </div>
  );
}

function SkippedGroupRow(props: { group: ReturnType<typeof groupSkippedRows>[number]; expanded: boolean; onToggle: () => void }) {
  const g = props.group;
  return (
    <tr className="audit-row-category audit-skipped-group-row">
      <td className="audit-category" colSpan={2}>
        <button type="button" className="audit-skipped-group-toggle" onClick={props.onToggle} aria-expanded={props.expanded}>
          <span className="audit-skipped-chevron" aria-hidden="true"><svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true" focusable="false"><path d="M6 4.5 10 8l-4 3.5V4.5z" /></svg></span>
          <span className="audit-category-main">{g.label}</span>
          {g.label === 'CustomCleanup' && <span className="scan-custom-path-tag muted">Custom Path</span>}
          <span className="audit-kind muted">· {g.rows.length} skipped ({g.reason || 'Skipped'})</span>
        </button>
      </td>
      <td className="audit-size num">{formatGroupBytes(g.rows)}</td>
      <td className="audit-outcome"><span className="badge outcome-pill outcome-skipped">Skipped</span></td>
    </tr>
  );
}

export function AuditTable(props: {
  auditRows: AuditRow[];
  visibleRows: AuditRow[];
  outcomeFilter: OutcomeFilter;
  onOutcome: (f: OutcomeFilter) => void;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  primaryLabel?: string;
  kind?: string;
}) {
  const categoryRows = props.visibleRows.filter((r) => r.kind === 'Category');
  const skippedRows = props.visibleRows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
  const otherRows = props.visibleRows.filter((r) => r.kind === 'File' && r.outcomeTone !== 'skipped');
  const groups = groupSkippedRows(skippedRows);
  const sectionTitle = (props.kind === 'scan' || props.primaryLabel === 'Space Found') ? 'SAFE CATEGORIES' : 'CLEANED CATEGORIES';
  return (
    <div className="log-structured audit-table-wrap logs-detail-fill">
      {props.auditRows.length > 0 && <OutcomeFilters filter={props.outcomeFilter} onChange={props.onOutcome} />}
      {props.auditRows.length === 0 ? <p className="muted">No entries.</p> : props.visibleRows.length === 0 ? <p className="muted">No entries for this filter.</p> : (
        <table className="table audit-log-table logs-detail-table logs-detail-table-grouped">
          <colgroup><col className="logs-col-category" /><col className="logs-col-location" /><col className="logs-col-size" /><col className="logs-col-outcome" /></colgroup>
          <thead><tr><th scope="col">Category / Target</th><th scope="col">Location</th><th scope="col" className="num logs-th-size">Size</th><th scope="col" className="logs-th-outcome">Outcome</th></tr></thead>
          <tbody>
            {categoryRows.length > 0 && <tr className="audit-section-row"><td colSpan={4}>{sectionTitle}</td></tr>}
            {categoryRows.map((r, i) => <AuditRowView key={`${i}-${r.location}-${r.outcome}-c`} row={r} indented={false} />)}
            {otherRows.map((r, i) => <AuditRowView key={`${i + 1000}-${r.location}-${r.outcome}-o`} row={r} indented={false} />)}
            {skippedRows.length > 0 && <tr className="audit-section-row"><td colSpan={4}>SKIPPED FILES</td></tr>}
            {groups.map((g, gi) => (
              <Fragment key={`sg-${g.key}`}>
                <SkippedGroupRow group={g} expanded={props.expanded.has(g.key)} onToggle={() => props.onToggle(g.key)} />
                {props.expanded.has(g.key) && g.rows.map((r, i) => <AuditRowView key={`${2000 + gi * 100 + i}-${r.location}-${r.outcome}-s`} row={r} indented={true} />)}
              </Fragment>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
