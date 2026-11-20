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

// App-shell side of the window-close handshake. The Rust handler always defers
// the native close and asks here; a clean draft is approved immediately with no
// prompt, while a dirty draft is persisted or the close is cancelled.
export function useCloseGuard({ bridge, isDirty, flushDraft }: CloseGuardDeps): CloseGuardState {
  const [promptOpen, setPromptOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [closePending, setClosePending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The generation of the request currently awaiting a decision. Kept in a ref
  // so a late answer always targets the request the user actually saw (#93).
  const requestIdRef = useRef<string | null>(null);

  const deliver = useCallback(
    async (approved: boolean) => {
      const requestId = requestIdRef.current;
      if (!requestId) return;
      // Lock draft controls at approval *intent*, before awaiting the IPC call:
      // Rust blocks that call until in-flight Cleans settle, and no edit may land
      // during that wait (#93).
      if (approved) setClosePending(true);
      try {
        await bridge.resolveClose(approved, requestId);
        requestIdRef.current = null;
        setPromptOpen(false);
        setError(null);
      } catch {
        // Keep the id so the user can re-deliver, surface the failure, reopen
        // the prompt and unlock controls. Never leave Rust silently pending.
        if (approved) setClosePending(false);
        setError(CLOSE_DELIVERY_ERROR);
        setPromptOpen(true);
      }
    },
    [bridge],
  );

  useEffect(
    () =>
      bridge.onHostCloseRequest((requestId) => {
        requestIdRef.current = requestId;
        setError(null);
        if (!isDirty()) {
          // Clean close: no dialog, no save round trip beyond the handshake.
          void deliver(true);
          return;
        }
        setPromptOpen(true);
      }),
    [bridge, deliver, isDirty],
  );

  // A permanently unavailable host listener is reported by the bridge after
  // its bounded retries. Surface it as a user-visible error; the bridge has
  // already cleared the native pending gate, so the draft is kept intact.
  useEffect(() => {
    const off = bridge.onRegistrationFailure?.((message) => setError(message));
    return () => off?.();
  }, [bridge]);

  // Complementary only. A browser or a close path that bypasses the Rust
  // handler still gets the standard "leave site?" protection when dirty; the
  // authoritative decision is always made by the Rust handshake above.
  useEffect(() => {
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      if (!isDirty()) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', onBeforeUnload);
    return () => window.removeEventListener('beforeunload', onBeforeUnload);
  }, [isDirty]);

  const cancel = useCallback(() => {
    // Escape and the Cancel button must be inert while a save is in flight;
    // otherwise the later approval could answer a newer request (#93).
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
  }, [deliver, flushDraft, saving]);

  return { promptOpen, saving, closePending, error, cancel, saveAndClose };
}
