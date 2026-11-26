import { Fragment } from 'react';
import type { AuditRow, OutcomeFilter } from './logParsers';
import { formatGroupBytes, groupSkippedRows } from './useLogsSelectionHelpers';

function auditRowClass(input: { row: AuditRow; indented: boolean }): string {
  const base = input.row.kind === 'Category' ? 'audit-row-category' : 'audit-row-file';
  if (input.indented) return base + ' audit-row-indented';
  return base;
}

function isUnknownOutcome(outcome: string): boolean {
  const value = outcome.toLowerCase();
  if (value.includes('unknown')) return true;
  return value.includes('unverified');
}

function outcomeBadgeClass(input: { row: AuditRow }): string {
  let cls = 'badge outcome-pill outcome-' + input.row.outcomeTone;
  if (isUnknownOutcome(input.row.outcome)) cls += ' badge-unknown';
  return cls;
}

function AuditRowView(props: { row: AuditRow; indented: boolean }) {
  const { row, indented } = props;
  const categoryClass = indented ? 'audit-category-cell audit-category-cell-indented' : 'audit-category-cell';
  return (
    <tr className={auditRowClass({ row, indented })}>
      <td className="audit-category"><div className={categoryClass}><span className="audit-category-main">{row.category}</span>{row.sublabel ? <span className="audit-kind muted">{row.sublabel}</span> : null}</div></td>
      <td className="audit-location mono" title={row.location}><bdi>{String(row.location || '')}</bdi></td>
      <td className="audit-size num">{row.size}</td>
      <td className="audit-outcome"><span className={outcomeBadgeClass({ row })}>{row.outcome}</span></td>
    </tr>
  );
}

function FilterButton(props: { active: boolean; label: string; onClick: () => void }) {
  return <button type="button" className={props.active ? 'logs-outcome-link active' : 'logs-outcome-link'} aria-pressed={props.active} onClick={props.onClick}>{props.label}</button>;
}

function OutcomeFilters(props: { filter: OutcomeFilter; onChange: (f: OutcomeFilter) => void }) {
  return (
    <div className="logs-outcome-filter" role="group" aria-label="Outcome filters">
      <div className="logs-outcome-inline-filters" role="group" aria-label="Outcome filters">
        <FilterButton active={props.filter === 'all'} label="All" onClick={() => props.onChange('all')} />
        <FilterButton active={props.filter === 'eligible'} label="Safe" onClick={() => props.onChange('eligible')} />
        <FilterButton active={props.filter === 'skipped'} label="Skipped" onClick={() => props.onChange('skipped')} />
      </div>
    </div>
  );
}

function SkippedChevron() {
  return <span className="audit-skipped-chevron" aria-hidden="true"><svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true" focusable="false"><path d="M6 4.5 10 8l-4 3.5V4.5z" /></svg></span>;
}

function SkippedGroupRow(props: { group: ReturnType<typeof groupSkippedRows>[number]; expanded: boolean; onToggle: () => void }) {
  const g = props.group;
  return (
    <tr className="audit-row-category audit-skipped-group-row">
      <td className="audit-category" colSpan={2}>
        <button type="button" className="audit-skipped-group-toggle" onClick={props.onToggle} aria-expanded={props.expanded}>
          <SkippedChevron />
          <span className="audit-category-main">{g.label}</span>
          {g.label === 'CustomCleanup' && <span className="scan-custom-path-tag muted">Custom Path</span>}
          <span className="audit-kind muted">{'· ' + String(g.rows.length) + ' skipped (' + (g.reason || 'Skipped') + ')'}</span>
        </button>
      </td>
      <td className="audit-size num">{formatGroupBytes(g.rows)}</td>
      <td className="audit-outcome"><span className="badge outcome-pill outcome-skipped">Skipped</span></td>
    </tr>
  );
}

function AuditTableHead() {
  return (
    <thead><tr><th scope="col">Category / Target</th><th scope="col">Location</th><th scope="col" className="num logs-th-size">Size</th><th scope="col" className="logs-th-outcome">Outcome</th></tr></thead>
  );
}

function AuditRowsList(props: { rows: AuditRow[]; keyOffset: number; keySuffix: string; indented: boolean }) {
  return <>{props.rows.map((r, i) => <AuditRowView key={String(i + props.keyOffset) + '-' + r.location + '-' + r.outcome + '-' + props.keySuffix} row={r} indented={props.indented} />)}</>;
}

function SkippedGroups(props: { groups: ReturnType<typeof groupSkippedRows>; expanded: Set<string>; onToggle: (key: string) => void }) {
  return <>{props.groups.map((g, gi) => (
    <Fragment key={'sg-' + g.key}>
      <SkippedGroupRow group={g} expanded={props.expanded.has(g.key)} onToggle={() => props.onToggle(g.key)} />
      {props.expanded.has(g.key) && <AuditRowsList rows={g.rows} keyOffset={2000 + gi * 100} keySuffix="s" indented={true} />}
    </Fragment>
  ))}</>;
}

function tableSectionTitle(input: { kind?: string; primaryLabel?: string }): string {
  if (input.kind === 'scan') return 'SAFE CATEGORIES';
  if (input.primaryLabel === 'Space Found') return 'SAFE CATEGORIES';
  return 'CLEANED CATEGORIES';
}

function AuditTableBody(props: { visibleRows: AuditRow[]; expanded: Set<string>; onToggle: (key: string) => void; sectionTitle: string }) {
  const categoryRows = props.visibleRows.filter((r) => r.kind === 'Category');
  const skippedRows = props.visibleRows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
  const otherRows = props.visibleRows.filter((r) => r.kind === 'File' && r.outcomeTone !== 'skipped');
  const groups = groupSkippedRows(skippedRows);
  return (
    <tbody>
      {categoryRows.length > 0 && <tr className="audit-section-row"><td colSpan={4}>{props.sectionTitle}</td></tr>}
      <AuditRowsList rows={categoryRows} keyOffset={0} keySuffix="c" indented={false} />
      <AuditRowsList rows={otherRows} keyOffset={1000} keySuffix="o" indented={false} />
      {skippedRows.length > 0 && <tr className="audit-section-row"><td colSpan={4}>SKIPPED FILES</td></tr>}
      <SkippedGroups groups={groups} expanded={props.expanded} onToggle={props.onToggle} />
    </tbody>
  );
}

function AuditContent(props: { auditRows: AuditRow[]; visibleRows: AuditRow[]; expanded: Set<string>; onToggle: (key: string) => void; sectionTitle: string }) {
  if (props.auditRows.length === 0) return <p className="muted">No entries.</p>;
  if (props.visibleRows.length === 0) return <p className="muted">No entries for this filter.</p>;
  return (
    <table className="table audit-log-table logs-detail-table logs-detail-table-grouped">
      <colgroup><col className="logs-col-category" /><col className="logs-col-location" /><col className="logs-col-size" /><col className="logs-col-outcome" /></colgroup>
      <AuditTableHead />
      <AuditTableBody visibleRows={props.visibleRows} expanded={props.expanded} onToggle={props.onToggle} sectionTitle={props.sectionTitle} />
    </table>
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
  const sectionTitle = tableSectionTitle({ kind: props.kind, primaryLabel: props.primaryLabel });
  return (
    <div className="log-structured audit-table-wrap logs-detail-fill">
      {props.auditRows.length > 0 && <OutcomeFilters filter={props.outcomeFilter} onChange={props.onOutcome} />}
      <AuditContent auditRows={props.auditRows} visibleRows={props.visibleRows} expanded={props.expanded} onToggle={props.onToggle} sectionTitle={sectionTitle} />
    </div>
  );
}