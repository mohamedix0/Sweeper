import { formatBytes, pluralize } from '../../utils/format';
import { deleteActionLabel } from '../cleanActionLabel';

interface AnalyzerCleanPanelProps {
  selectedCount: number;
  selectedBytes: string;
  hasPreview: boolean;
  hasSelectedRisky: boolean;
  cleaning: boolean;
  previewLoading: boolean;
  canPreview: boolean;
  canClean: boolean;
  onClear: () => void;
  onPreview: () => void;
  onClean: () => void;
}

function reviewLoadingLabel(args: AnalyzerCleanPanelProps): string | null {
  if (args.previewLoading) return 'Generating Preview…';
  return null;
}

function reviewEmptyLabel(args: AnalyzerCleanPanelProps): string | null {
  if (args.selectedCount <= 0) return 'Review & Clean';
  return null;
}

function reviewLabel(args: AnalyzerCleanPanelProps): string {
  const loading = reviewLoadingLabel(args);
  if (loading) return loading;
  const empty = reviewEmptyLabel(args);
  if (empty) return empty;
  return `Review & Clean (${formatBytes({ bytes: args.selectedBytes })})`;
}

function cleanLabel(args: { cleaning: boolean; count: number; size: string }): string {
  return deleteActionLabel({
    count: args.count,
    size: args.size,
    cleaning: args.cleaning,
    singular: 'item',
    plural: 'items',
  });
}

function reviewClassName(args: { selected: boolean }): string {
  if (args.selected) return 'btn primary';
  return 'btn';
}

function UnselectAllButton(props: { selected: number; cleaning: boolean; onClear: () => void }) {
  if (props.selected <= 0) return null;
  return <button type="button" className="btn small subtle-btn cleanup-deselect-btn" onClick={props.onClear} disabled={props.cleaning} aria-label="Unselect all">Unselect all</button>;
}

function RiskyHint(props: { risky: boolean }) {
  if (!props.risky) return null;
  return <span className="scan-bottom-bar-hint muted">Includes Caution/Unknown items · requires DELETE confirm</span>;
}

function ActionButton(props: AnalyzerCleanPanelProps) {
  if (props.hasPreview) {
    return <button className="btn danger primary" onClick={props.onClean} disabled={!props.canClean} aria-label="Clean Permanently">{cleanLabel({ cleaning: props.cleaning, count: props.selectedCount, size: formatBytes({ bytes: props.selectedBytes }) })}</button>;
  }
  const cls = reviewClassName({ selected: props.selectedCount > 0 });
  return <button className={cls} onClick={props.onPreview} disabled={!props.canPreview} aria-label="Review & Clean">{reviewLabel(props)}</button>;
}

export function AnalyzerCleanPanel(props: AnalyzerCleanPanelProps) {
  const count = pluralize({ count: props.selectedCount, singular: 'item', plural: 'items' });
  const size = formatBytes({ bytes: props.selectedBytes });
  return (
    <div className="scan-bottom-bar" aria-label="Analyzer actions">
      <div className="scan-bottom-bar-left">
        <span className="scan-bottom-bar-summary"><strong className="num">{count} selected</strong> · <span className="num">{size}</span></span>
        <UnselectAllButton selected={props.selectedCount} cleaning={props.cleaning} onClear={props.onClear} />
        <RiskyHint risky={props.hasSelectedRisky} />
      </div>
      <div className="scan-bottom-bar-right">
        <ActionButton {...props} />
      </div>
    </div>
  );
}
