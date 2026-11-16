import { formatBytes, pluralize } from '../../utils/format';

export function ScanCleanBar(props: {
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
}) {
  return (
    <div className="scan-bottom-bar" aria-label="Cleanup actions">
      <div className="scan-bottom-bar-left">
        <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(props.selectedIdsCount, 'file', 'files')} selected</strong> ({pluralize(props.selectedCategoryCount, 'category', 'categories')}) · <span className="num">{formatBytes(props.selectedBytes)}</span> reclaimable</span>
        {props.selectedIdsCount > 0 && <button type="button" className="btn small subtle-btn" onClick={props.onClear} disabled={props.cleaning}>Clear</button>}
        <span className="scan-bottom-bar-hint muted">Next: Review summary · permanent delete (skips Recycle Bin)</span>
      </div>
      <div className="scan-bottom-bar-right">
        {!props.hasPreview ? (
          <button className="btn primary" onClick={props.onPreview} disabled={!props.canPreview} aria-label="Review & Clean">{props.previewLoading ? 'Generating Preview…' : `Review & Clean${props.selectedIdsCount > 0 ? ` (${formatBytes(props.selectedBytes)})` : ''}`}</button>
        ) : (
          <button className="btn danger primary" onClick={props.onClean} disabled={!props.canClean} aria-label="Clean Permanently">{props.cleaning ? 'Cleaning…' : 'Clean Permanently'}</button>
        )}
      </div>
    </div>
  );
}
