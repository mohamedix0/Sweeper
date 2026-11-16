import { formatBytes, pluralize } from '../../utils/format';

export function AnalyzerCleanPanel(props: {
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
}) {
  return (
    <div className="scan-bottom-bar" aria-label="Analyzer actions">
      <div className="scan-bottom-bar-left">
        <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(props.selectedCount, 'item', 'items')} selected</strong> · <span className="num">{formatBytes(props.selectedBytes)}</span></span>
        {props.selectedCount > 0 && <button type="button" className="btn small subtle-btn" onClick={props.onClear} disabled={props.cleaning}>Clear</button>}
        {props.hasSelectedRisky && <span className="scan-bottom-bar-hint muted">Includes Caution/Unknown items · requires DELETE confirm</span>}
      </div>
      <div className="scan-bottom-bar-right">
        {!props.hasPreview ? (
          <button className={`btn ${props.selectedCount > 0 ? 'primary' : ''}`} onClick={props.onPreview} disabled={!props.canPreview} aria-label="Review & Clean">{props.previewLoading ? 'Generating Preview…' : `Review & Clean${props.selectedCount > 0 ? ` (${formatBytes(props.selectedBytes)})` : ''}`}</button>
        ) : (
          <button className="btn danger primary" onClick={props.onClean} disabled={!props.canClean} aria-label="Clean Permanently">{props.cleaning ? 'Cleaning…' : 'Clean Permanently'}</button>
        )}
      </div>
    </div>
  );
}
