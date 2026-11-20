import type { CloseBridge } from './closeBridge';

// Test-only CloseBridge fixture shared by the app-shell and hook-level window
// close tests. It records every decision (and its request generation) the
// webview sends to Rust and exposes a way to drive the host request without a
// Tauri runtime.
export type FakeCloseBridge = CloseBridge & {
  decisions: boolean[];
  decisionIds: string[];
  requestHostClose: (requestId?: string) => void;
};

export function fakeCloseBridge(): FakeCloseBridge {
  const listeners: Array<(requestId: string) => void> = [];
  const decisions: boolean[] = [];
  const decisionIds: string[] = [];
  return {
    decisions,
    decisionIds,
    onHostCloseRequest(handler) {
      listeners.push(handler);
      return () => {
        const index = listeners.indexOf(handler);
        if (index >= 0) listeners.splice(index, 1);
      };
    },
    async resolveClose(approved, requestId) {
      decisions.push(approved);
      decisionIds.push(requestId);
    },
    requestHostClose(requestId = 'close-1') {
      for (const listener of [...listeners]) listener(requestId);
    },
  };
}
