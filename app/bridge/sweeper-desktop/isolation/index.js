// Tauri isolation application script (Phase 2 read-only bridge).
// Validates/forwards IPC payloads through the isolation boundary.
window.__TAURI_ISOLATION_HOOK__ = (payload) => {
  return payload;
};
