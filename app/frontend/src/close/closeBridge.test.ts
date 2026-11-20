import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  CLOSE_REGISTRATION_ERROR,
  createCloseBridge,
  type CloseBridgeDeps,
  type CloseReadyPayload,
} from './closeBridge';

function setTauriRuntime(present: boolean): void {
  const globals = window as unknown as Record<string, unknown>;
  if (present) globals.__TAURI_INTERNALS__ = {};
  else delete globals.__TAURI_INTERNALS__;
}

function timers(): CloseBridgeDeps['schedule'] {
  return (fn, ms) => {
    window.setTimeout(fn, ms);
  };
}

function noPending(): CloseReadyPayload {
  return { pendingRequest: null };
}

function depsWith(
  overrides: Partial<CloseBridgeDeps>,
  observed: {
    decisions?: boolean[];
    resets?: number;
    ready?: CloseReadyPayload;
  } = {},
): CloseBridgeDeps {
  return {
    listenForHostClose: async () => () => {},
    resolve: async (approved) => {
      observed.decisions?.push(approved);
    },
    notifyListenerReady: async () => observed.ready ?? noPending(),
    resetPendingClose: async () => {
      observed.resets = (observed.resets ?? 0) + 1;
    },
    schedule: timers(),
    ...overrides,
  };
}

describe('close listener registration (#93)', () => {
  afterEach(() => {
    setTauriRuntime(false);
    vi.useRealTimers();
  });

  it('retries a transient listener failure with backoff and then receives host closes', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const state: { wired: ((payload: { requestId: string }) => void) | null } = { wired: null };
    const decisions: boolean[] = [];
    const listen = vi.fn((handler: (payload: { requestId: string }) => void): Promise<() => void> => {
      if (listen.mock.calls.length < 3) return Promise.reject(new Error('ipc not ready'));
      state.wired = handler;
      return Promise.resolve(() => {});
    });
    const bridge = createCloseBridge(
      depsWith({ listenForHostClose: listen, resolve: async (approved) => { decisions.push(approved); } }),
    );
    const received = vi.fn();
    const off = bridge.onHostCloseRequest(received);

    await vi.advanceTimersByTimeAsync(2000);

    expect(listen).toHaveBeenCalledTimes(3);
    state.wired?.({ requestId: 'close-9' });
    expect(received).toHaveBeenCalledWith('close-9');
    // A recovered listener uses the normal handshake: no fallback cancel.
    expect(decisions).toEqual([]);
    off();
  });

  it('a persistently failing listener clears the native pending gate and reports an error', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const observed: { decisions: boolean[]; resets: number } = { decisions: [], resets: 0 };
    const failures: string[] = [];
    const listen = vi.fn((): Promise<() => void> => Promise.reject(new Error('ipc down')));
    const bridge = createCloseBridge(
      depsWith({ listenForHostClose: listen }, observed),
    );
    const offFailure = bridge.onRegistrationFailure?.((message) => failures.push(message)) ?? (() => {});
    const off = bridge.onHostCloseRequest(() => {});

    await vi.advanceTimersByTimeAsync(12000);

    // Bounded retries were attempted before recovery ran.
    expect(listen.mock.calls.length).toBeGreaterThanOrEqual(4);
    // The native pending gate is explicitly re-armed; resolve(false) is not a
    // substitute because the pending request was never received here.
    expect(observed.resets).toBeGreaterThanOrEqual(1);
    expect(observed.decisions).toEqual([]);
    expect(failures).toContain(CLOSE_REGISTRATION_ERROR);
    off();
    offFailure();
  });

  it('delivers a pending request when the listener becomes ready after the close', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const state: { wired: ((payload: { requestId: string }) => void) | null } = { wired: null };
    const listen = vi.fn((handler: (payload: { requestId: string }) => void): Promise<() => void> => {
      if (listen.mock.calls.length < 3) return Promise.reject(new Error('ipc not ready'));
      state.wired = handler;
      return Promise.resolve(() => {});
    });
    const observed = { ready: { pendingRequest: { requestId: 'pending-7' } } };
    const bridge = createCloseBridge(
      depsWith(
        { listenForHostClose: listen },
        { ready: observed.ready },
      ),
    );
    const received = vi.fn();
    const off = bridge.onHostCloseRequest(received);

    await vi.advanceTimersByTimeAsync(2000);

    // No new host event was dispatched: the registration-time pending query
    // delivered the close that arrived while no listener existed, and did so
    // without approving it (#93).
    expect(received).toHaveBeenCalledWith('pending-7');
    expect(listen).toHaveBeenCalledTimes(3);
    off();
  });

  it('re-arms after exhausted retries and delivers the pending close on re-registration', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const state: { wired: ((payload: { requestId: string }) => void) | null } = { wired: null };
    const listen = vi.fn((handler: (payload: { requestId: string }) => void): Promise<() => void> => {
      // First full cycle fails; the recovery re-registration succeeds.
      if (listen.mock.calls.length <= 4) return Promise.reject(new Error('ipc down'));
      state.wired = handler;
      return Promise.resolve(() => {});
    });
    const observed: { resets: number; ready: CloseReadyPayload } = {
      resets: 0,
      ready: { pendingRequest: { requestId: 'pending-recovery' } },
    };
    const bridge = createCloseBridge(
      depsWith({ listenForHostClose: listen }, observed),
    );
    const received = vi.fn();
    const off = bridge.onHostCloseRequest(received);

    await vi.advanceTimersByTimeAsync(5000);

    expect(observed.resets).toBeGreaterThanOrEqual(1);
    expect(received).toHaveBeenCalledWith('pending-recovery');
    off();
  });
});

describe('close decision delivery (#93)', () => {
  afterEach(() => {
    setTauriRuntime(false);
  });

  it('retries a transient delivery failure before succeeding', async () => {
    setTauriRuntime(true);
    const resolve = vi.fn(async () => {
      if (resolve.mock.calls.length < 3) throw new Error('ipc not ready');
    });
    const bridge = createCloseBridge(depsWith({ resolve }));

    await expect(bridge.resolveClose(true, 'close-2')).resolves.toBeUndefined();
    expect(resolve).toHaveBeenCalledTimes(3);
    expect(resolve).toHaveBeenLastCalledWith(true, 'close-2');
  });

  it('surfaces exhaustion so the caller can reopen the prompt for re-delivery', async () => {
    setTauriRuntime(true);
    const resolve = vi.fn(async () => {
      throw new Error('ipc down');
    });
    const bridge = createCloseBridge(depsWith({ resolve }));

    await expect(bridge.resolveClose(false, 'close-3')).rejects.toThrow('ipc down');
    expect(resolve).toHaveBeenCalledTimes(3);
  });
});
