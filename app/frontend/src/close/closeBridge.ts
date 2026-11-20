import { commands, events } from '../contract/bindings';

// The typed bridge to the Rust `CloseRequested` handshake. Kept behind a small
// interface so component tests can drive the host request and inspect the
// decision without a Tauri runtime.
export type CloseBridge = {
  /** Subscribe to the host's deferred-close request; returns an unsubscribe. */
  onHostCloseRequest: (handler: () => void) => () => void;
  /** Answer the pending host request. Only an approval lets the process exit. */
  resolveClose: (approved: boolean) => Promise<void>;
};

// Browser-preview fallback: the mock backend already runs the whole app in a
// plain browser, so mirror the two host events as DOM CustomEvents there. In a
// real Tauri window the typed IPC events are the only channel.
const BROWSER_CLOSE_REQUEST = 'sweeper:close-requested';

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function createCloseBridge(): CloseBridge {
  return {
    onHostCloseRequest(handler) {
      if (!isTauriRuntime()) {
        const listener = () => handler();
        window.addEventListener(BROWSER_CLOSE_REQUEST, listener);
        return () => window.removeEventListener(BROWSER_CLOSE_REQUEST, listener);
      }
      let disposed = false;
      let unlisten: (() => void) | null = null;
      void events.closeRequestedByHost
        .listen(() => {
          if (!disposed) handler();
        })
        .then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        })
        .catch(() => {
          // Event channel not ready yet; a later real close is still handled.
        });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
    async resolveClose(approved) {
      if (!isTauriRuntime()) {
        // Exposed for browser-preview diagnostics of the handshake.
        (window as unknown as { sweeperCloseDecision?: boolean }).sweeperCloseDecision = approved;
        return;
      }
      const result = await commands.resolveClose(approved);
      if (result.status === 'error') throw result.error;
    },
  };
}
