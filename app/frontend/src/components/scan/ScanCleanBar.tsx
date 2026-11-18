import { formatBytes, pluralize } from '../../utils/format';

interface ScanCleanBarProps {
  selectedIdsCount: number;
  selectedCategoryCount: number;
  selectedBytes: string | bigint;
  cleaning: boolean;
  previewLoading: boolean;
  hasPreview: boolean;
  canPreview: boolean;
  canClean: boolean;
  onClear: () => void;
  onPreview: () => void;
  onClean: () => void;
}

function reviewLabel(args: ScanCleanBarProps): string {
  if (args.previewLoading) return 'Generating Preview…';
  if (args.selectedIdsCount <= 0) return 'Review & Clean';
  return `Review & Clean (${formatBytes({ bytes: args.selectedBytes })})`;
}

function cleanLabel(args: { cleaning: boolean }): string {
  if (args.cleaning) return 'Cleaning…';
  return 'Clean Permanently';
}

function ActionButton(props: ScanCleanBarProps) {
  if (!props.hasPreview) {
    return <button className="btn primary" onClick={props.onPreview} disabled={!props.canPreview} aria-label="Review & Clean">{reviewLabel(props)}</button>;
  }
  return <button className="btn danger primary" onClick={props.onClean} disabled={!props.canClean} aria-label="Clean Permanently">{cleanLabel({ cleaning: props.cleaning })}</button>;
}

function UnselectAllButton(props: ScanCleanBarProps) {
  if (props.selectedIdsCount <= 0) return null;
  return <button type="button" className="btn small subtle-btn" onClick={props.onClear} disabled={props.cleaning} aria-label="Unselect all">Unselect all</button>;
}

export function ScanCleanBar(props: ScanCleanBarProps) {
  const files = pluralize({ count: props.selectedIdsCount, singular: 'file', plural: 'files' });
  const cats = pluralize({ count: props.selectedCategoryCount, singular: 'category', plural: 'categories' });
  const size = formatBytes({ bytes: props.selectedBytes });
  return (
    <div className="scan-bottom-bar" aria-label="Cleanup actions">
      <div className="scan-bottom-bar-left">
        <span className="scan-bottom-bar-summary"><strong className="num">{files} selected</strong> ({cats}) · <span className="num">{size}</span> reclaimable</span>
        <UnselectAllButton {...props} />
        <span className="scan-bottom-bar-hint muted">Next: Review summary · permanent delete (skips Recycle Bin)</span>
      </div>
      <div className="scan-bottom-bar-right"><ActionButton {...props} /></div>
    </div>
  );
}
