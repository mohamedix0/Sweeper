import { useRef } from 'react';
import { ConfirmDialogShell } from './ConfirmDialogShell';

// Confirmation shown when the user tries to leave Settings with an unsaved
// draft. The safe action ("Keep editing") is focused by default so a stray
// Enter/Escape never discards work; discarding always needs an explicit click.
export function UnsavedSettingsDialog(props: {
  onKeepEditing: () => void;
  onDiscard: () => void;
}) {
  const keepRef = useRef<HTMLButtonElement>(null);
  const discardRef = useRef<HTMLButtonElement>(null);

  return (
    <ConfirmDialogShell
      titleId="unsaved-settings-title"
      messageId="unsaved-settings-message"
      title="Unsaved settings changes"
      message="You have unsaved changes in Settings. Leaving this view will discard them. Keep editing, or discard the changes and leave?"
      onEscape={props.onKeepEditing}
      initialFocusRef={keepRef}
      trapOrder={[keepRef, discardRef]}
    >
      <button ref={keepRef} className="btn" onClick={props.onKeepEditing}>Keep editing</button>
      <button ref={discardRef} className="btn danger" onClick={props.onDiscard}>Discard changes</button>
    </ConfirmDialogShell>
  );
}
