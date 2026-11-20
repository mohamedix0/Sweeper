import { useEffect, useRef, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { trapTab } from './dialogFocus';

// Shared shell for the app's confirmation alerts. It owns the behaviour that
// must be identical in every dialog — capturing/restoring focus, trapping Tab,
// treating Escape as the safe action, and the backdrop/markup — so individual
// dialogs only supply their copy, action buttons and initial focus.
//
// Disabled actions are never focus targets. While `busy` (e.g. saving) the
// dialog container itself is the in-dialog focus fallback, so Tab cannot escape
// to the page behind the modal, and focus is restored to an enabled action only
// on the busy -> idle transition, not on ordinary rerenders (#93).
export function ConfirmDialogShell(props: {
  titleId: string;
  messageId: string;
  title: ReactNode;
  message: ReactNode;
  onEscape: () => void;
  initialFocusRef: RefObject<HTMLButtonElement>;
  trapOrder: RefObject<HTMLButtonElement>[];
  error?: string | null;
  busy?: boolean;
  children: ReactNode;
}) {
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const trapOrderRef = useRef(props.trapOrder);
  trapOrderRef.current = props.trapOrder;
  const wasBusy = useRef(props.busy ?? false);

  const enabledActions = () =>
    trapOrderRef.current
      .map((ref) => ref.current)
      .filter((el): el is HTMLButtonElement => el !== null && !el.disabled);

  const focusables = (): HTMLElement[] => {
    const actions = enabledActions();
    return actions.length > 0
      ? actions
      : [dialogRef.current].filter((el): el is HTMLDivElement => el !== null);
  };

  useEffect(() => {
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    const initial = props.initialFocusRef.current;
    const target =
      initial && !initial.disabled ? initial : (enabledActions()[0] ?? dialogRef.current);
    target?.focus();
    return () => {
      previouslyFocused.current?.focus();
    };
    // Refs are stable for the dialog's lifetime; re-running on every render
    // would steal focus back while the user is interacting with the actions.
  }, [props.initialFocusRef]);

  useEffect(() => {
    const busy = props.busy ?? false;
    if (busy && !wasBusy.current) {
      // Both actions are disabled while busy; keep focus inside the dialog so
      // the Tab trap (which lives on the dialog) keeps receiving key events.
      dialogRef.current?.focus();
    } else if (!busy && wasBusy.current) {
      enabledActions()[0]?.focus();
      if (!dialogRef.current?.contains(document.activeElement as Node | null)) {
        dialogRef.current?.focus();
      }
    }
    wasBusy.current = busy;
  }, [props.busy]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      props.onEscape();
      return;
    }
    if (event.key !== 'Tab') return;
    trapTab(event, focusables());
  };

  return (
    <div className="dialog-backdrop">
      <div
        ref={dialogRef}
        tabIndex={-1}
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
