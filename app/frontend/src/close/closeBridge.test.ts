import { afterEach, describe, expect, it, vi } from 'vitest';
import { CLOSE_REGISTRATION_ERROR, createCloseBridge, type CloseBridgeDeps } from './closeBridge';

function setTauriRuntime(present: boolean): void {
  const globals = window as unknown as Record<string, unknown>;
  if (present) globals.__TAURI_INTERNALS__ = {};
  else delete globals.__TAURI_INTERNALS__;
}

function timers(): CloseBridgeDeps['schedule'] {
  return (fn, ms) => { window.setTimeout(fn, ms); };
}

describe('close listener registration (#93)', () => {
  afterEach(() => {
    setTauriRuntime(false);
    vi.useRealTimers();
  });

  it('retries a transient listener failure with backoff and then receives host closes', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const state: { wired: (() => void) | null } = { wired: null };
    const decisions: boolean[] = [];
    const listen = vi.fn((handler: () => void): Promise<() => void> => {
      if (listen.mock.calls.length < 3) return Promise.reject(new Error('ipc not ready'));
      state.wired = handler;
      return Promise.resolve(() => {});
    });
    const bridge = createCloseBridge({
      listenForHostClose: listen,
      resolve: async (approved) => { decisions.push(approved); },
      schedule: timers(),
    });
    const received = vi.fn();
    const off = bridge.onHostCloseRequest(received);

    await vi.advanceTimersByTimeAsync(2000);

    expect(listen).toHaveBeenCalledTimes(3);
    state.wired?.();
    expect(received).toHaveBeenCalledTimes(1);
    // A recovered listener uses the normal handshake: no fallback cancel.
    expect(decisions).toEqual([]);
    off();
  });

  it('a permanently failing listener cancels the close and reports an error instead of hanging the app', async () => {
    vi.useFakeTimers();
    setTauriRuntime(true);
    const decisions: boolean[] = [];
    const failures: string[] = [];
    const listen = vi.fn((): Promise<() => void> => Promise.reject(new Error('ipc down')));
    const bridge = createCloseBridge({
      listenForHostClose: listen,
      resolve: async (approved) => { decisions.push(approved); },
      schedule: timers(),
    });
    const offFailure = bridge.onRegistrationFailure?.((message) => failures.push(message)) ?? (() => {});
    bridge.onHostCloseRequest(() => {});

    await vi.advanceTimersByTimeAsync(5000);

    // Bounded retries were attempted before the fallback ran.
    expect(listen.mock.calls.length).toBeGreaterThanOrEqual(2);
    // Rust receives a Cancelled decision, so the deferred close is answered
    // rather than left in `awaiting`: the app stays open and usable.
    expect(decisions).toEqual([false]);
    // Never auto-approve, so no edit can be silently dropped.
    expect(decisions).not.toContain(true);
    expect(failures).toEqual([CLOSE_REGISTRATION_ERROR]);
    offFailure();
  });
});
