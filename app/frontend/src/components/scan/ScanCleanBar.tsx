import type { PreviewResult } from '../cleanSharedCore';
import { formatBytes, pluralize } from '../../utils/format';
import { UnselectAllButton } from '../UnselectAllButton';
import { deleteActionLabel, localizedCount } from '../cleanActionLabel';

export interface ScanCleanBarProps {
  /** Original selection totals (pre-Preview summary). */
  selectedIdsCount: number;
  /** The validated Preview totals; post-Preview actions must use these, never the original selection. */
  preview?: PreviewResult | null;
  selectedCategoryCount: number;
  selectedBytes: string | bigint;
  cleaning: boolean;
  previewLoading: boolean;
  hasPreview: boolean;
  canPreview: boolean;
  canClean: boolean;
  reviewPending: boolean;
  onClear: () => void;
  onPreview: () => void;
  onClean: () => void;
}

function reviewLabel(args: ScanCleanBarProps): string {
  if (args.previewLoading) return 'Generating Preview…';
  if (args.selectedIdsCount <= 0) return 'Review & Clean';
  return `Review & Clean (${formatBytes({ bytes: args.selectedBytes })})`;
}

function ActionButton(props: ScanCleanBarProps) {
  if (!props.hasPreview) {
    return <button className="btn primary" onClick={props.onPreview} disabled={!props.canPreview} aria-label="Review & Clean">{reviewLabel(props)}</button>;
  }
  const label = deleteActionLabel({
    count: props.preview ? Number(props.preview.totalSelectedCount) : props.selectedIdsCount,
    size: formatBytes({ bytes: props.preview ? props.preview.totalSelectedBytes : props.selectedBytes }),
    cleaning: props.cleaning,
    singular: 'file',
    plural: 'files',
  });
  return (
    <button
      className="btn danger primary"
      onClick={props.onClean}
      disabled={!props.canClean}
      aria-describedby={props.reviewPending ? 'clean-review-pending' : undefined}
    >
      {label}
    </button>
  );
}

function PendingReviewStatus(props: { count: number }) {
  return (
    <span id="clean-review-pending" className="clean-review-pending" role="status" aria-live="polite">
      Preparing safety review… {localizedCount({ count: props.count, singular: 'item', plural: 'items' })}
    </span>
  );
}

export function ScanCleanBar(props: ScanCleanBarProps) {
  const files = pluralize({ count: props.selectedIdsCount, singular: 'file', plural: 'files' });
  const cats = pluralize({ count: props.selectedCategoryCount, singular: 'category', plural: 'categories' });
  const size = formatBytes({ bytes: props.selectedBytes });
  return (
    <div className="scan-bottom-bar" aria-label="Cleanup actions">
      <div className="scan-bottom-bar-left">
        <span className="scan-bottom-bar-summary"><strong className="num">{files} selected</strong> ({cats}) · <span className="num">{size}</span> reclaimable</span>
        <UnselectAllButton selected={props.selectedIdsCount} cleaning={props.cleaning} onClear={props.onClear} />
        <span className="scan-bottom-bar-hint muted">Next: Review summary · permanent delete (skips Recycle Bin)</span>
      </div>
      <div className="scan-bottom-bar-right">
        {props.hasPreview && props.reviewPending && <PendingReviewStatus count={props.selectedIdsCount} />}
        <ActionButton {...props} />
      </div>
    </div>
  );
}
