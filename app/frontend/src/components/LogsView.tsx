import { pluralize } from '../utils/format';
import type { SweeperClient } from '../api/client';
import { LogsDetailsPane } from './logs/LogsDetailsPane';
import { LogsEntryList } from './logs/LogsEntryList';
import { LogsFilterBar } from './logs/LogsFilterBar';
import { useLogsSelection } from './logs/useLogsSelection';

interface LogsViewProps {
  client: SweeperClient;
  focusLogId?: string | null;
  onFocusConsumed?: () => void;
  onScanAgain?: () => void;
}

export function LogsView(props: LogsViewProps) {
  const w = useLogsSelection({ client: props.client, focusLogId: props.focusLogId, onFocusConsumed: props.onFocusConsumed });
  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header" aria-label="Activity History header">
        <div className="page-header-left"><h1 className="page-title" id="logs-heading">Activity History</h1><span className="header-subtitle num">{pluralize({ count: w.logs.length, singular: 'activity', plural: 'activities' })}</span></div>
      </header>
      {w.error && <div className="notice error" role="alert">{w.error}</div>}
      <section className="logs-layout" aria-label="Activity history">
        <div className="logs-pane logs-pane-left">
          <div className="logs-pane-header logs-pane-header-stack">
            <h2 className="pane-section-title">Recent Activity</h2>
            <LogsFilterBar filter={w.filter} query={w.query} onFilter={w.setFilter} onQuery={w.setQuery} />
          </div>
          <div className="logs-pane-body">
            <LogsEntryList logs={w.logs} filtered={w.filteredLogs} contentById={w.contentById} selectedId={w.selected?.id ?? null} selectedContent={w.selected?.content ?? null} onOpen={(id) => void w.openLog(id)} />
          </div>
        </div>
        <LogsDetailsPane
          selected={w.selected}
          meta={w.selectedMeta}
          summary={w.selectedSummary}
          activity={w.activitySummary}
          mode={w.mode}
          copied={w.copied}
          auditRows={w.auditRows}
          visibleRows={w.visibleAuditRows}
          outcomeFilter={w.outcomeFilter}
          expanded={w.expandedSkippedGroups}
          hasLogs={w.logs.length > 0}
          hasFilteredLogs={w.filteredLogs.length > 0}
          onMode={w.setMode}
          onCopy={() => void w.copyLog()}
          onExport={w.exportLog}
          onOutcome={w.setOutcomeFilter}
          onToggle={w.toggleSkippedGroup}
          onScanAgain={props.onScanAgain}
        />
      </section>
    </div>
  );
}
