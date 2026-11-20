import { commands, events } from '../contract/bindings';

// The typed bridge to the Rust `CloseRequested` handshake. Kept behind a small
// interface so component tests can drive the host request and inspect the
// decision without a Tauri runtime.
//
// This is deliberately a dedicated host-lifecycle bridge rather than part of
// the SweeperClient facade in api/client.ts. The facade (and its mock) model the
// domain API used by every view; this handshake is a narrow window-lifecycle
// concern (one command + one event) that must work before a client exists and
// must be replaceable in close tests. Routing it through the facade would force
// the mock controller to grow a second, unrelated protocol with no safety gain.
export type CloseBridge = {
  /** Subscribe to the host's deferred-close request; returns an unsubscribe. */
  onHostCloseRequest: (handler: () => void) => () => void;
  /** Answer the pending host request. Only an approval lets the process exit. */
  resolveClose: (approved: boolean) => Promise<void>;
  /** Notified when the host listener cannot be registered after all retries. */
  onRegistrationFailure?: (handler: (message: string) => void) => () => void;
};

// User-facing message for a permanently unavailable close listener. The app is
// deliberately left open with the draft intact; nothing is auto-approved.
export const CLOSE_REGISTRATION_ERROR =
  'The close handler could not be registered, so Sweeper was kept open and your changes were not discarded. Save your work and try closing again.';

// Browser-preview fallback: the mock backend already runs the whole app in a
// plain browser, so mirror the two host events as DOM CustomEvents there. In a
// real Tauri window the typed IPC events are the only channel.
const BROWSER_CLOSE_REQUEST = 'sweeper:close-requested';

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export type CloseBridgeDeps = {
  listenForHostClose: (handler: () => void) => Promise<() => void>;
  resolve: (approved: boolean) => Promise<void>;
  schedule: (fn: () => void, delayMs: number) => void;
};

// A transient "IPC not ready" rejection during startup must not permanently
// lose the host listener: with Rust always preventing the native close and
// waiting for a webview decision, a missing listener would hang the app.
const LISTEN_ATTEMPTS = 4;
const LISTEN_BACKOFF_MS = [0, 100, 400, 1000];

function defaultDeps(): CloseBridgeDeps {
  return {
    listenForHostClose: (handler) => events.closeRequestedByHost.listen(() => handler()),
    resolve: async (approved) => {
      const result = await commands.resolveClose(approved);
      if (result.status === 'error') throw result.error;
    },
    schedule: (fn, delayMs) => { window.setTimeout(fn, delayMs); },
  };
}

// Browser-preview path: mirror the two host events as DOM CustomEvents. The
// Tauri runtime never takes this branch; the typed IPC events are its only
// channel.
function registerBrowserCloseListener(handler: () => void): () => void {
  const listener = () => handler();
  window.addEventListener(BROWSER_CLOSE_REQUEST, listener);
  return () => window.removeEventListener(BROWSER_CLOSE_REQUEST, listener);
}

// Delay before the retry at `attemptIndex`. Clamped to the last configured
// delay so a growing index can never produce an undefined/NaN timer.
function retryDelay(attemptIndex: number): number {
  return LISTEN_BACKOFF_MS[attemptIndex] ?? LISTEN_BACKOFF_MS[LISTEN_BACKOFF_MS.length - 1];
}

// Failure path after bounded retries are exhausted: answer the deferred host
// close with a cancel (never an approval) so Rust records a Cancelled decision
// instead of hanging in `awaiting`, then surface an actionable error. The draft
// stays intact — a dirty close is never auto-approved.
function cancelAndReportFailure(
  resolve: CloseBridgeDeps['resolve'],
  notifyFailure: () => void,
): void {
  void resolve(false).catch(() => {});
  notifyFailure();
}

// Register the Tauri host listener, retrying transient "IPC not ready"
// rejections with bounded backoff. Returns a disposer that detaches the
// listener and stops any pending retries.
function startHostCloseListener(
  handler: () => void,
  deps: CloseBridgeDeps,
  notifyFailure: () => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;

  const attempt = (index: number) => {
    if (disposed) return;
    void Promise.resolve()
      .then(() => deps.listenForHostClose(() => { if (!disposed) handler(); }))
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        if (disposed) return;
        const next = index + 1;
        if (next < LISTEN_ATTEMPTS) {
          deps.schedule(() => attempt(next), retryDelay(next));
          return;
        }
        cancelAndReportFailure(deps.resolve, notifyFailure);
      });
  };
  attempt(0);

  return () => {
    disposed = true;
    unlisten?.();
  };
}

export function createCloseBridge(overrides: Partial<CloseBridgeDeps> = {}): CloseBridge {
  const deps: CloseBridgeDeps = { ...defaultDeps(), ...overrides };
  const failureHandlers = new Set<(message: string) => void>();

  const notifyFailure = () => {
    for (const handler of [...failureHandlers]) handler(CLOSE_REGISTRATION_ERROR);
  };

  return {
    onHostCloseRequest(handler) {
      return isTauriRuntime()
        ? startHostCloseListener(handler, deps, notifyFailure)
        : registerBrowserCloseListener(handler);
    },
    async resolveClose(approved) {
      if (!isTauriRuntime()) {
        // Exposed for browser-preview diagnostics of the handshake.
        (window as unknown as { sweeperCloseDecision?: boolean }).sweeperCloseDecision = approved;
        return;
      }
      await deps.resolve(approved);
    },
    onRegistrationFailure(handler) {
      failureHandlers.add(handler);
      return () => { failureHandlers.delete(handler); };
    },
  };
}
