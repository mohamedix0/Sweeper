import { commands, events } from '../contract/bindings';

// The typed bridge to the Rust `CloseRequested` handshake. Kept behind a small
// interface so component tests can drive the host request and inspect the
// decision without a Tauri runtime.
//
// This is deliberately a dedicated host-lifecycle bridge rather than part of
// the SweeperClient facade in api/client.ts. The facade (and its mock) model the
// domain API used by every view; this handshake is a narrow window-lifecycle
// concern that must work before a client exists and must be replaceable in close
// tests. Routing it through the facade would force the mock controller to grow a
// second, unrelated protocol with no safety gain.
//
// Protocol (#93):
// - every host request carries a generation id that decisions echo back;
// - a newly registered listener queries for a pending request and delivers it;
// - an unreachable listener clears the native pending gate and re-arms;
// - decision delivery is retried with a bound and surfaces exhaustion.

export type CloseRequestPayload = { requestId: string };
export type CloseReadyPayload = { pendingRequest: CloseRequestPayload | null };

export type CloseBridge = {
  /** Subscribe to the host's deferred-close request; returns an unsubscribe. */
  onHostCloseRequest: (handler: (requestId: string) => void) => () => void;
  /** Answer the pending host request. Only an approval lets the process exit. */
  resolveClose: (approved: boolean, requestId: string) => Promise<void>;
  /** Notified when the host listener cannot be registered after all retries. */
  onRegistrationFailure?: (handler: (message: string) => void) => () => void;
};

// User-facing messages. The app is deliberately left open with the draft
// intact; nothing is auto-approved.
export const CLOSE_REGISTRATION_ERROR =
  'The close handler could not be registered, so Sweeper was kept open and your changes were not discarded. Save your work and try closing again.';
export const CLOSE_DELIVERY_ERROR =
  'The close decision could not be delivered, so Sweeper was kept open and your changes were not discarded. Please try closing again.';

// Browser-preview fallback: the mock backend already runs the whole app in a
// plain browser, so mirror the host event as a DOM CustomEvent there. In a real
// Tauri window the typed IPC events are the only channel.
const BROWSER_CLOSE_REQUEST = 'sweeper:close-requested';
const BROWSER_REQUEST_ID = 'browser';

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export type CloseBridgeDeps = {
  listenForHostClose: (handler: (payload: CloseRequestPayload) => void) => Promise<() => void>;
  resolve: (approved: boolean, requestId: string) => Promise<void>;
  notifyListenerReady: () => Promise<CloseReadyPayload>;
  resetPendingClose: () => Promise<void>;
  schedule: (fn: () => void, delayMs: number) => void;
};

// A transient "IPC not ready" rejection during startup must not permanently
// lose the host listener: with Rust always preventing the native close and
// waiting for a webview decision, a missing listener would hang the app.
const LISTEN_ATTEMPTS = 4;
const LISTEN_BACKOFF_MS = [0, 100, 400, 1000];
const MAX_RECOVERIES = 2;
const RECOVERY_RETRY_MS = 2000;
const RESOLVE_ATTEMPTS = 3;

function defaultDeps(): CloseBridgeDeps {
  return {
    listenForHostClose: (handler) =>
      events.closeRequestedByHost.listen((event) => handler(event.payload)),
    resolve: async (approved, requestId) => {
      const result = await commands.resolveClose(approved, requestId);
      if (result.status === 'error') throw result.error;
    },
    notifyListenerReady: async () => {
      const result = await commands.notifyCloseListenerReady();
      if (result.status === 'error') throw result.error;
      return result.data;
    },
    resetPendingClose: async () => {
      const result = await commands.resetPendingClose();
      if (result.status === 'error') throw result.error;
    },
    schedule: (fn, delayMs) => {
      window.setTimeout(fn, delayMs);
    },
  };
}

// Browser-preview path: mirror the host event as a DOM CustomEvent. The Tauri
// runtime never takes this branch; the typed IPC events are its only channel.
function registerBrowserCloseListener(handler: (requestId: string) => void): () => void {
  const listener = () => handler(BROWSER_REQUEST_ID);
  window.addEventListener(BROWSER_CLOSE_REQUEST, listener);
  return () => window.removeEventListener(BROWSER_CLOSE_REQUEST, listener);
}

// Delay before the retry at `attemptIndex`. Clamped to the last configured
// delay so a growing index can never produce an undefined/NaN timer.
function retryDelay(attemptIndex: number): number {
  return LISTEN_BACKOFF_MS[attemptIndex] ?? LISTEN_BACKOFF_MS[LISTEN_BACKOFF_MS.length - 1];
}

// Clear the native pending gate after an unreachable listener. `resolve(false)`
// cannot be relied on to cancel a pending request we never received, so use the
// explicit recovery command. A failure here is best-effort: the user-visible
// error still tells them Sweeper stayed open (#93).
async function recoverPendingGate(deps: CloseBridgeDeps): Promise<void> {
  try {
    await deps.resetPendingClose();
  } catch {
    /* best effort; the error notice is authoritative */
  }
}

// Deliver a decision idempotently with bounded retry. Rust ignores a replayed
// decision for the same generation, so retrying is safe. Exhaustion rejects so
// the hook can surface the failure and reopen the prompt for re-delivery (#93).
async function deliverWithRetry(
  deps: CloseBridgeDeps,
  approved: boolean,
  requestId: string,
): Promise<void> {
  let lastError: unknown;
  for (let attempt = 0; attempt < RESOLVE_ATTEMPTS; attempt += 1) {
    try {
      await deps.resolve(approved, requestId);
      return;
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}

// Mutable state shared by the registration helpers. Kept in one object so the
// disposal guard and the retry counters stay consistent across async callbacks.
type ListenerSession = {
  disposed: boolean;
  unlisten: (() => void) | null;
  recoveryTimer: number | null;
  recoveries: number;
};

// A newly live listener answers a request that arrived before registration.
// A failed pending query must never create a duplicate subscription, so the
// native gate is cleared instead (#93).
function deliverPendingRequest(
  deps: CloseBridgeDeps,
  handler: (requestId: string) => void,
  isLive: () => boolean,
): Promise<void> {
  return deps
    .notifyListenerReady()
    .then((ready) => {
      if (isLive() && ready.pendingRequest) {
        handler(ready.pendingRequest.requestId);
      }
    })
    .catch(() => {
      void recoverPendingGate(deps);
    });
}

// Store a registered listener (or tear it down if disposal raced the
// registration) and drain any pending host request through the live listener.
function onListenerRegistered(
  session: ListenerSession,
  deps: CloseBridgeDeps,
  handler: (requestId: string) => void,
  unlisten: () => void,
): void {
  if (session.disposed) {
    unlisten();
    return;
  }
  session.unlisten = unlisten;
  void deliverPendingRequest(deps, handler, () => !session.disposed);
}

// Related state and callbacks for the registration-failure path, bundled so the
// helper takes a single named value instead of a long positional argument list.
type RegistrationFailureContext = {
  session: ListenerSession;
  deps: CloseBridgeDeps;
  index: number;
  scheduleAttempt: (next: number) => void;
  retryFromZero: () => void;
  notifyFailure: () => void;
};

// Once registration retries are exhausted, clear the native gate, surface the
// failure, and schedule a bounded number of later re-registrations so a
// subsequent close reaches a live listener (#93).
function handleRegistrationFailure({
  session,
  deps,
  index,
  scheduleAttempt,
  retryFromZero,
  notifyFailure,
}: RegistrationFailureContext): void {
  if (session.disposed) return;
  const next = index + 1;
  if (next < LISTEN_ATTEMPTS) {
    scheduleAttempt(next);
    return;
  }
  void recoverPendingGate(deps).then(() => {
    if (session.disposed) return;
    notifyFailure();
    if (session.recoveries < MAX_RECOVERIES) {
      session.recoveries += 1;
      session.recoveryTimer = window.setTimeout(retryFromZero, RECOVERY_RETRY_MS);
    }
  });
}

// Register the Tauri host listener with bounded backoff for transient
// "IPC not ready" rejections; the helpers above own the success, pending-query,
// and failure/recovery paths (#93).
function startHostCloseListener(
  handler: (requestId: string) => void,
  deps: CloseBridgeDeps,
  notifyFailure: () => void,
): () => void {
  const session: ListenerSession = {
    disposed: false,
    unlisten: null,
    recoveryTimer: null,
    recoveries: 0,
  };

  const attempt = (index: number) => {
    if (session.disposed) return;
    void Promise.resolve()
      .then(() =>
        deps.listenForHostClose((payload) => {
          if (!session.disposed) handler(payload.requestId);
        }),
      )
      .then((unlisten) => onListenerRegistered(session, deps, handler, unlisten))
      .catch(() =>
        handleRegistrationFailure({
          session,
          deps,
          index,
          scheduleAttempt: (next) => deps.schedule(() => attempt(next), retryDelay(next)),
          retryFromZero: () => attempt(0),
          notifyFailure,
        }),
      );
  };
  attempt(0);

  return () => {
    session.disposed = true;
    session.unlisten?.();
    if (session.recoveryTimer !== null) window.clearTimeout(session.recoveryTimer);
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
    async resolveClose(approved, requestId) {
      if (!isTauriRuntime()) {
        // Exposed for browser-preview diagnostics of the handshake.
        (window as unknown as { sweeperCloseDecision?: boolean }).sweeperCloseDecision = approved;
        return;
      }
      await deliverWithRetry(deps, approved, requestId);
    },
    onRegistrationFailure(handler) {
      failureHandlers.add(handler);
      return () => {
        failureHandlers.delete(handler);
      };
    },
  };
}
