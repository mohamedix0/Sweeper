import type { CloseBridge } from './closeBridge';

// Test-only CloseBridge fixture shared by the app-shell and hook-level window
// close tests. It records every decision the webview sends to Rust and exposes
// a way to drive the host request without a Tauri runtime.
export type FakeCloseBridge = CloseBridge & {
  decisions: boolean[];
  requestHostClose: () => void;
};

export function fakeCloseBridge(): FakeCloseBridge {
  const listeners: Array<() => void> = [];
  const decisions: boolean[] = [];
  return {
    decisions,
    onHostCloseRequest(handler) {
      listeners.push(handler);
      return () => {
        const index = listeners.indexOf(handler);
        if (index >= 0) listeners.splice(index, 1);
      };
    },
    async resolveClose(approved) {
      decisions.push(approved);
    },
    requestHostClose() {
      for (const listener of [...listeners]) listener();
    },
  };
}
