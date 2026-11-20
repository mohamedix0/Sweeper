import { useMemo, useRef } from 'react';
import { ConfirmDialogShell } from './ConfirmDialogShell';

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
  // Stable order so the shell's busy-transition effect does not rerun on every
  // saving/error rerender and steal focus back (#93).
  const trapOrder = useMemo(() => [cancelRef, saveRef], []);

  return (
    <ConfirmDialogShell
      titleId="close-draft-title"
      messageId="close-draft-message"
      title="Unsaved settings changes"
      message="You have unsaved changes in Settings. Save them before closing, or cancel to keep Sweeper open."
      onEscape={props.onCancel}
      initialFocusRef={cancelRef}
      trapOrder={trapOrder}
      error={props.error}
      busy={props.saving}
    >
      <button ref={cancelRef} className="btn" onClick={props.onCancel} disabled={props.saving}>Cancel</button>
      <button ref={saveRef} className="btn primary" onClick={props.onSaveAndClose} disabled={props.saving}>
        {props.saving ? 'Saving…' : 'Save & close'}
      </button>
    </ConfirmDialogShell>
  );
}
