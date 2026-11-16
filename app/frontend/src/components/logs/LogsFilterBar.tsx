import type { LogFilter } from './logParsers';

export function LogsFilterBar(props: { filter: LogFilter; query: string; onFilter: (f: LogFilter) => void; onQuery: (q: string) => void }) {
  return (
    <div className="logs-filter-bar">
      <div className="segmented-toggle scan-status-segmented" role="group" aria-label="Activity filters">
        <button type="button" className={`seg-btn ${props.filter === 'all' ? 'active' : ''}`} aria-pressed={props.filter === 'all'} onClick={() => props.onFilter('all')}>All</button>
        <button type="button" className={`seg-btn ${props.filter === 'scan' ? 'active' : ''}`} aria-pressed={props.filter === 'scan'} onClick={() => props.onFilter('scan')}>Scans</button>
        <button type="button" className={`seg-btn ${props.filter === 'clean' ? 'active' : ''}`} aria-pressed={props.filter === 'clean'} onClick={() => props.onFilter('clean')}>Cleanups</button>
      </div>
      <input type="search" className="logs-search" placeholder="Search activity..." value={props.query} onChange={(e) => props.onQuery(e.target.value)} aria-label="Search activity history" />
    </div>
  );
}
