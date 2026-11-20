import { useEffect, useRef } from 'react';

// Confirmation shown when the user tries to leave Settings with an unsaved
// draft. The safe action ("Keep editing") is focused by default so a stray
// Enter/Escape never discards work; discarding always needs an explicit click.
export function UnsavedSettingsDialog(props: {
  onKeepEditing: () => void;
  onDiscard: () => void;
}) {
  const keepRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    keepRef.current?.focus();
  }, []);
  return (
    <div className="dialog-backdrop">
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="unsaved-settings-title"
        aria-describedby="unsaved-settings-message"
        onKeyDown={(event) => {
          if (event.key === 'Escape') props.onKeepEditing();
        }}
      >
        <h2 id="unsaved-settings-title" className="confirm-dialog-title">Unsaved settings changes</h2>
        <p id="unsaved-settings-message" className="confirm-dialog-message">
          You have unsaved changes in Settings. Leaving this view will discard them. Keep editing, or discard the changes and leave?
        </p>
        <div className="confirm-dialog-actions">
          <button ref={keepRef} className="btn" onClick={props.onKeepEditing}>Keep editing</button>
          <button className="btn danger" onClick={props.onDiscard}>Discard changes</button>
        </div>
      </div>
    </div>
  );
}
