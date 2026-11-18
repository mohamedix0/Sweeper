import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

// The Tauri isolation script is a classic browser script (no module exports), so
// load it into a tiny sandbox with only a `window` global and exercise the hook
// it installs. This keeps the production script unchanged and dependency-free.
const ISOLATION_SCRIPT = resolve(
  process.cwd(),
  '..',
  'bridge',
  'sweeper-desktop',
  'isolation',
  'index.js',
);

type IsolationHook = (message: unknown) => unknown;

function loadHook(): IsolationHook {
  const source = readFileSync(ISOLATION_SCRIPT, 'utf8');
  const sandboxWindow: { __TAURI_ISOLATION_HOOK__?: IsolationHook } = {};
  runInNewContext(source, { window: sandboxWindow });
  const hook = sandboxWindow.__TAURI_ISOLATION_HOOK__;
  expect(typeof hook).toBe('function');
  return hook as IsolationHook;
}

function invoke(cmd: string, payload: unknown = {}) {
  return { cmd, callback: 1, error: 2, options: {}, payload };
}

const VALID_ARGUMENTS: Record<string, unknown> = {
  start_scan: { tiers: null },
  get_scan_results: { sessionId: 'session-1' },
  preview_clean: { sessionId: 'session-1', itemIds: ['t1::1'], confirmations: null },
  execute_clean: { previewToken: 'token-1' },
  start_analyzer: { root: null },
  cancel_analyzer: { sessionId: 'session-1' },
  explain_item: { itemId: 't1::1' },
  get_settings: {},
  update_settings: { settings: { enableTier1: true } },
  validate_custom_path: { path: 'some-path' },
  list_logs: {},
  read_log: { logId: 'clean-1' },
  get_elevation_status: {},
  relaunch_elevated: {},
};

describe('Tauri isolation IPC boundary', () => {
  it('forwards all 14 allow-listed app commands with well-shaped arguments', () => {
    const hook = loadHook();
    expect(Object.keys(VALID_ARGUMENTS)).toHaveLength(14);
    for (const [cmd, payload] of Object.entries(VALID_ARGUMENTS)) {
      const message = invoke(cmd, payload);
      expect(hook(message), `command ${cmd} should be forwarded`).toEqual(message);
    }
  });

  it('forwards Tauri event-plugin messages used for progress subscriptions', () => {
    const hook = loadHook();
    const message = invoke('plugin:event|listen', { event: 'scan-progress' });
    expect(hook(message)).toEqual(message);
  });

  it('drops an unknown command instead of forwarding it', () => {
    const hook = loadHook();
    expect(() => hook(invoke('delete_all_the_things'))).toThrow(/unknown command/);
  });

  it('drops known commands whose argument shapes are wrong', () => {
    const hook = loadHook();
    expect(() => hook(invoke('execute_clean', {}))).toThrow(/invalid arguments/);
    expect(() => hook(invoke('read_log', { logId: 42 }))).toThrow(/invalid arguments/);
    expect(() =>
      hook(invoke('preview_clean', { sessionId: 'session-1', itemIds: 'not-an-array' })),
    ).toThrow(/invalid arguments/);
  });

  it('drops malformed messages with no command name', () => {
    const hook = loadHook();
    expect(() => hook({ callback: 1, error: 2, payload: {} })).toThrow(/missing command name/);
    expect(() => hook(null)).toThrow(/malformed message/);
  });
});
