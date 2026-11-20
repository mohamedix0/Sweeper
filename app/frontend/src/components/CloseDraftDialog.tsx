import { useEffect, useRef, type KeyboardEvent } from 'react';
import { trapTab } from './dialogFocus';

// Shown only when the host has asked to close and the mounted Settings draft is
// dirty. Closing must not lose the edit: the safe action ("Cancel") is focused
// by default, and saving is the only way to complete a dirty close.
export function CloseDraftDialog(props: {
  saving: boolean;
  error: string | null;
  onCancel: () => void;
  onSaveAndClose: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const saveRef = useRef<HTMLButtonElement>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    return () => {
      previouslyFocused.current?.focus();
    };
  }, []);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      props.onCancel();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = [cancelRef.current, saveRef.current].filter(
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
        aria-labelledby="close-draft-title"
        aria-describedby="close-draft-message"
        onKeyDown={handleKeyDown}
      >
        <h2 id="close-draft-title" className="confirm-dialog-title">Unsaved settings changes</h2>
        <p id="close-draft-message" className="confirm-dialog-message">
          You have unsaved changes in Settings. Save them before closing, or cancel to keep Sweeper open.
        </p>
        {props.error && <p className="notice error" role="alert">{props.error}</p>}
        <div className="confirm-dialog-actions">
          <button ref={cancelRef} className="btn" onClick={props.onCancel} disabled={props.saving}>Cancel</button>
          <button ref={saveRef} className="btn primary" onClick={props.onSaveAndClose} disabled={props.saving}>
            {props.saving ? 'Saving…' : 'Save & close'}
          </button>
        </div>
      </div>
    </div>
  );
}
