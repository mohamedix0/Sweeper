import { useCallback, useEffect, useState } from 'react';
import type { CloseBridge } from './closeBridge';

export type CloseGuardDeps = {
  bridge: CloseBridge;
  isDirty: () => boolean;
  flushDraft: () => Promise<boolean>;
};

export type CloseGuardState = {
  promptOpen: boolean;
  saving: boolean;
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
  const [error, setError] = useState<string | null>(null);

  useEffect(
    () =>
      bridge.onHostCloseRequest(() => {
        setError(null);
        if (!isDirty()) {
          // Clean close: no dialog, no save round trip beyond the handshake.
          void bridge.resolveClose(true).catch(() => {});
          return;
        }
        setPromptOpen(true);
      }),
    [bridge, isDirty],
  );

  // A permanently unavailable host listener is reported by the bridge after
  // its bounded retries. Surface it as a user-visible error; the bridge has
  // already cancelled the deferred close, so the draft is kept intact.
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
    setPromptOpen(false);
    setError(null);
    void bridge.resolveClose(false).catch(() => {});
  }, [bridge]);

  const saveAndClose = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      const saved = await flushDraft();
      if (!saved) {
        setError(SAVE_FAILED_MESSAGE);
        return;
      }
      await bridge.resolveClose(true);
      setPromptOpen(false);
    } catch {
      setError(SAVE_FAILED_MESSAGE);
    } finally {
      setSaving(false);
    }
  }, [bridge, flushDraft]);

  return { promptOpen, saving, error, cancel, saveAndClose };
}
