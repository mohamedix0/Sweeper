import { useEffect, useRef, type KeyboardEvent } from 'react';
import { trapTab } from './dialogFocus';

// Confirmation shown when the user tries to leave Settings with an unsaved
// draft. The safe action ("Keep editing") is focused by default so a stray
// Enter/Escape never discards work; discarding always needs an explicit click.
export function UnsavedSettingsDialog(props: {
  onKeepEditing: () => void;
  onDiscard: () => void;
}) {
  const keepRef = useRef<HTMLButtonElement>(null);
  const discardRef = useRef<HTMLButtonElement>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);

  useEffect(() => {
    // Capture where focus came from, then move it into the dialog. Restoring on
    // unmount covers both close paths (Keep editing and Discard changes).
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    keepRef.current?.focus();
    return () => {
      previouslyFocused.current?.focus();
    };
  }, []);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      props.onKeepEditing();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = [keepRef.current, discardRef.current].filter(
      (el): el is HTMLButtonElement => el !== null,
    );
    trapTab(event, focusables);
  };

  return (
    <div className="dialog-backdrop">
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="unsaved-settings-title"
        aria-describedby="unsaved-settings-message"
        onKeyDown={handleKeyDown}
      >
        <h2 id="unsaved-settings-title" className="confirm-dialog-title">Unsaved settings changes</h2>
        <p id="unsaved-settings-message" className="confirm-dialog-message">
          You have unsaved changes in Settings. Leaving this view will discard them. Keep editing, or discard the changes and leave?
        </p>
        <div className="confirm-dialog-actions">
          <button ref={keepRef} className="btn" onClick={props.onKeepEditing}>Keep editing</button>
          <button ref={discardRef} className="btn danger" onClick={props.onDiscard}>Discard changes</button>
        </div>
      </div>
    </div>
  );
}
