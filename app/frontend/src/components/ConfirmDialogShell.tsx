import {
  useCallback,
  useEffect,
  useRef,
  type KeyboardEvent,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
} from 'react';
import { trapTab } from './dialogFocus';

type ActionOrder = RefObject<HTMLButtonElement>[];

// Disabled actions are never focus targets. These are pure reads of the live
// refs so every focus path below shares exactly the same notion of "focusable".
function enabledActions(order: ActionOrder): HTMLButtonElement[] {
  return order
    .map((ref) => ref.current)
    .filter((el): el is HTMLButtonElement => el !== null && !el.disabled);
}

// While busy both actions are disabled, so the dialog container itself is the
// in-dialog fallback; Tab then cannot escape to the page behind the modal.
function focusables(order: ActionOrder, dialogRef: RefObject<HTMLDivElement>): HTMLElement[] {
  const actions = enabledActions(order);
  return actions.length > 0
    ? actions
    : [dialogRef.current].filter((el): el is HTMLDivElement => el !== null);
}

// Capture focus on mount and restore it on unmount. Refs are stable for the
// dialog's lifetime, so re-running on every render would steal focus back while
// the user is interacting with the actions.
function useFocusCapture(
  dialogRef: RefObject<HTMLDivElement>,
  orderRef: MutableRefObject<ActionOrder>,
  initialFocusRef: RefObject<HTMLButtonElement>,
): void {
  useEffect(() => {
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const initial = initialFocusRef.current;
    const target =
      initial && !initial.disabled
        ? initial
        : (enabledActions(orderRef.current)[0] ?? dialogRef.current);
    target?.focus();
    return () => {
      previouslyFocused?.focus();
    };
  }, [dialogRef, orderRef, initialFocusRef]);
}

// Keep focus inside the dialog across a busy transition, and restore it to an
// enabled action only on busy -> idle — never on an ordinary rerender.
function useBusyFocusTransition(
  dialogRef: RefObject<HTMLDivElement>,
  orderRef: MutableRefObject<ActionOrder>,
  busy: boolean,
): void {
  const wasBusy = useRef(busy);
  useEffect(() => {
    if (busy && !wasBusy.current) {
      dialogRef.current?.focus();
    } else if (!busy && wasBusy.current) {
      enabledActions(orderRef.current)[0]?.focus();
      if (!dialogRef.current?.contains(document.activeElement as Node | null)) {
        dialogRef.current?.focus();
      }
    }
    wasBusy.current = busy;
  }, [busy, dialogRef, orderRef]);
}

// Escape is the safe action; Tab is trapped inside the dialog's focusables.
function useDialogKeyTrap(
  dialogRef: RefObject<HTMLDivElement>,
  orderRef: MutableRefObject<ActionOrder>,
  onEscape: () => void,
): (event: KeyboardEvent<HTMLDivElement>) => void {
  return useCallback(
    (event) => {
      if (event.key === 'Escape') {
        onEscape();
        return;
      }
      if (event.key !== 'Tab') return;
      trapTab(event, focusables(orderRef.current, dialogRef));
    },
    [onEscape, dialogRef, orderRef],
  );
}

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
  trapOrder: ActionOrder;
  error?: string | null;
  busy?: boolean;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const trapOrderRef = useRef(props.trapOrder);
  trapOrderRef.current = props.trapOrder;
  const busy = props.busy ?? false;

  useFocusCapture(dialogRef, trapOrderRef, props.initialFocusRef);
  useBusyFocusTransition(dialogRef, trapOrderRef, busy);
  const handleKeyDown = useDialogKeyTrap(dialogRef, trapOrderRef, props.onEscape);

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
