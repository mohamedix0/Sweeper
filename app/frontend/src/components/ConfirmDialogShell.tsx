import { useEffect, useRef, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { trapTab } from './dialogFocus';

// Shared shell for the app's confirmation alerts. It owns the behaviour that
// must be identical in every dialog — capturing/restoring focus, trapping Tab,
// treating Escape as the safe action, and the backdrop/markup — so individual
// dialogs only supply their copy, action buttons and initial focus.
export function ConfirmDialogShell(props: {
  titleId: string;
  messageId: string;
  title: ReactNode;
  message: ReactNode;
  onEscape: () => void;
  initialFocusRef: RefObject<HTMLButtonElement>;
  trapOrder: RefObject<HTMLButtonElement>[];
  error?: string | null;
  children: ReactNode;
}) {
  const previouslyFocused = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    props.initialFocusRef.current?.focus();
    return () => {
      previouslyFocused.current?.focus();
    };
    // Refs are stable for the dialog's lifetime; re-running on every render
    // would steal focus back while the user is interacting with the actions.
  }, [props.initialFocusRef]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      props.onEscape();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = props.trapOrder
      .map((ref) => ref.current)
      .filter((el): el is HTMLButtonElement => el !== null);
    trapTab(event, focusables);
  };

  return (
    <div className="dialog-backdrop">
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={props.titleId}
        aria-describedby={props.messageId}
        onKeyDown={handleKeyDown}
      >
        <h2 id={props.titleId} className="confirm-dialog-title">{props.title}</h2>
        <p id={props.messageId} className="confirm-dialog-message">{props.message}</p>
        {props.error && <p className="notice error" role="alert">{props.error}</p>}
        <div className="confirm-dialog-actions">{props.children}</div>
      </div>
    </div>
  );
}
