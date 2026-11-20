import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef, useState } from 'react';
import { describe, expect, it } from 'vitest';
import { ConfirmDialogShell } from './ConfirmDialogShell';

function BusyDialog({ busy, note }: { busy: boolean; note: string }) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const saveRef = useRef<HTMLButtonElement>(null);
  return (
    <ConfirmDialogShell
      titleId="t"
      messageId="m"
      title="Title"
      message="Message"
      onEscape={() => {}}
      initialFocusRef={cancelRef}
      trapOrder={[cancelRef, saveRef]}
      error={note}
      busy={busy}
    >
      <button ref={cancelRef} disabled={busy}>Cancel</button>
      <button ref={saveRef} disabled={busy}>Save</button>
    </ConfirmDialogShell>
  );
}

describe('ConfirmDialogShell focus handling (#93)', () => {
  it('skips disabled actions, keeps Tab inside the dialog while busy, and restores focus only on busy->idle', async () => {
    const user = userEvent.setup();
    function Harness() {
      const [busy, setBusy] = useState(true);
      const [note, setNote] = useState('');
      return (
        <>
          <button onClick={() => setBusy(false)}>finish</button>
          <button onClick={() => setNote('note ' + note.length)}>bump</button>
          <BusyDialog busy={busy} note={note} />
        </>
      );
    }
    render(<Harness />);
    const dialog = screen.getByRole('alertdialog');
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    const save = screen.getByRole('button', { name: 'Save' });

    // Both actions are disabled; focus must sit on the in-dialog fallback, not
    // on a disabled button and not outside the dialog.
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(cancel);
    expect(document.activeElement).not.toBe(save);

    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);
    await user.tab({ shift: true });
    expect(dialog.contains(document.activeElement)).toBe(true);

    // Busy -> idle restores focus to an enabled action.
    await user.click(screen.getByRole('button', { name: 'finish' }));
    expect(document.activeElement).toBe(cancel);

    // An ordinary rerender (error/note change) must not steal focus back.
    save.focus();
    await user.click(screen.getByRole('button', { name: 'bump' }));
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'bump' }));
  });
});
