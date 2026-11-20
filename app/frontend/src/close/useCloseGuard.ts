import { useCallback, useEffect, useRef, useState } from 'react';
import { CLOSE_DELIVERY_ERROR, type CloseBridge } from './closeBridge';

export type CloseGuardDeps = {
  bridge: CloseBridge;
  isDirty: () => boolean;
  flushDraft: () => Promise<boolean>;
};

export type CloseGuardState = {
  promptOpen: boolean;
  saving: boolean;
  /** True from an approved decision through process exit: draft controls lock. */
  closePending: boolean;
  error: string | null;
  cancel: () => void;
  saveAndClose: () => Promise<void>;
};

const SAVE_FAILED_MESSAGE =
  'Settings could not be saved. The window was kept open and your changes are kept.';

// Owns the generation id and decision delivery. The id lives in a ref so a late
// answer always targets the request the user actually saw (#93). All callbacks
// are stable for the hook's lifetime, so `deliver` does not re-subscribe the
// host listener on every render.
function useDecisionDelivery(
  bridge: CloseBridge,
  setClosePending: (value: boolean) => void,
  setPromptOpen: (value: boolean) => void,
  setError: (message: string | null) => void,
) {
  const requestIdRef = useRef<string | null>(null);

  const lockClose = useCallback(() => setClosePending(true), [setClosePending]);
  const onDelivered = useCallback(() => {
    setPromptOpen(false);
    setError(null);
  }, [setPromptOpen, setError]);
  const onDeliveryFailed = useCallback(
    (approved: boolean) => {
      // Keep the id so the user can re-deliver; never leave Rust silently
      // pending. Surface the failure, reopen the prompt and unlock controls.
      if (approved) setClosePending(false);
      setError(CLOSE_DELIVERY_ERROR);
      setPromptOpen(true);
    },
    [setClosePending, setError, setPromptOpen],
  );

  const deliver = useCallback(
    async (approved: boolean) => {
      const requestId = requestIdRef.current;
      if (!requestId) return;
      // Lock draft controls at approval *intent*, before awaiting the IPC call:
      // Rust blocks that call until in-flight Cleans settle, and no edit may land
      // during that wait (#93).
      if (approved) lockClose();
      try {
        await bridge.resolveClose(approved, requestId);
        requestIdRef.current = null;
        onDelivered();
      } catch {
        onDeliveryFailed(approved);
      }
    },
    [bridge, lockClose, onDelivered, onDeliveryFailed],
  );

  const setRequestId = useCallback((requestId: string) => {
    requestIdRef.current = requestId;
  }, []);

  return { deliver, setRequestId };
}

// Subscribes to the host handshake and the listener-registration failure
// notice. A clean draft is approved at once; a dirty one opens the prompt.
function useHostCloseHandshake(
  bridge: CloseBridge,
  isDirty: () => boolean,
  setRequestId: (requestId: string) => void,
  deliver: (approved: boolean) => Promise<void>,
  setError: (message: string | null) => void,
  setPromptOpen: (value: boolean) => void,
): void {
  useEffect(
    () =>
      bridge.onHostCloseRequest((requestId) => {
        setRequestId(requestId);
        setError(null);
        if (!isDirty()) {
          // Clean close: no dialog, no save round trip beyond the handshake.
          void deliver(true);
          return;
        }
        setPromptOpen(true);
      }),
    [bridge, deliver, isDirty, setError, setPromptOpen, setRequestId],
  );

  // A permanently unavailable host listener is reported after its bounded
  // retries. The bridge has already cleared the native pending gate, so the
  // draft stays intact; only a user-visible error is needed here.
  useEffect(() => {
    const off = bridge.onRegistrationFailure?.((message) => setError(message));
    return () => off?.();
  }, [bridge, setError]);
}

// Complementary only. A browser or a close path that bypasses the Rust handler
// still gets the standard "leave site?" protection when dirty; the authoritative
// decision is always made by the Rust handshake.
function useBeforeUnloadGuard(isDirty: () => boolean): void {
  useEffect(() => {
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      if (!isDirty()) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', onBeforeUnload);
    return () => window.removeEventListener('beforeunload', onBeforeUnload);
  }, [isDirty]);
}

// The user actions. Cancel is inert while saving so a stale cancel cannot
// answer a newer close request; saving flushes every outstanding draft change
// before approving, and a failed save keeps the prompt open for a retry.
function useCloseActions(
  deliver: (approved: boolean) => Promise<void>,
  flushDraft: () => Promise<boolean>,
  saving: boolean,
  setSaving: (value: boolean) => void,
  setError: (message: string | null) => void,
): Pick<CloseGuardState, 'cancel' | 'saveAndClose'> {
  const cancel = useCallback(() => {
    if (saving) return;
    void deliver(false);
  }, [deliver, saving]);

  const saveAndClose = useCallback(async () => {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      const saved = await flushDraft();
      if (!saved) {
        // Keep the Rust request pending and the prompt open: the user may retry
        // the save or cancel. Preserve the existing save-failure behaviour.
        setError(SAVE_FAILED_MESSAGE);
        return;
      }
      // flushDraft settles every outstanding save and flushes a corrective
      // change before resolving, so a stale render cannot approve a dirty draft.
      // The prompt also traps focus while saving, so no edit can land here (#93).
      await deliver(true);
    } catch {
      setError(SAVE_FAILED_MESSAGE);
    } finally {
      setSaving(false);
    }
  }, [deliver, flushDraft, saving, setError, setSaving]);

  return { cancel, saveAndClose };
}

// App-shell side of the window-close handshake. The Rust handler always defers
// the native close and asks here; a clean draft is approved immediately with no
// prompt, while a dirty draft is persisted or the close is cancelled.
export function useCloseGuard({ bridge, isDirty, flushDraft }: CloseGuardDeps): CloseGuardState {
  const [promptOpen, setPromptOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [closePending, setClosePending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const { deliver, setRequestId } = useDecisionDelivery(
    bridge,
    setClosePending,
    setPromptOpen,
    setError,
  );

  useHostCloseHandshake(bridge, isDirty, setRequestId, deliver, setError, setPromptOpen);
  useBeforeUnloadGuard(isDirty);

  const { cancel, saveAndClose } = useCloseActions(
    deliver,
    flushDraft,
    saving,
    setSaving,
    setError,
  );

  return { promptOpen, saving, closePending, error, cancel, saveAndClose };
}
