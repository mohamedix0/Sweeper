/**
 * Shared deselect control used by the Scan bottom bar and the Analyzer panel.
 * The accessible name and DOM classes are part of the test contract — keep them
 * identical across both call sites.
 */
export interface UnselectAllButtonProps {
  selected: number;
  cleaning: boolean;
  onClear: () => void;
}

export function UnselectAllButton(props: UnselectAllButtonProps) {
  if (props.selected <= 0) return null;
  return <button type="button" className="btn small subtle-btn cleanup-deselect-btn" onClick={props.onClear} disabled={props.cleaning} aria-label="Unselect all">Unselect all</button>;
}
