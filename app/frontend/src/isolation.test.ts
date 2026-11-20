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
// Generated contract bindings (single source of truth for the command surface).
const BINDINGS_FILE = resolve(process.cwd(), 'src', 'contract', 'bindings.ts');

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
  resolve_close: { approved: true },
};

/** Command names generated into bindings.ts (the contract's IPC surface). */
function contractCommandNames(): Set<string> {
  const source = readFileSync(BINDINGS_FILE, 'utf8');
  return new Set(
    [...source.matchAll(/TAURI_INVOKE\(\s*"([a-z][a-z0-9_]*)"/g)].map((match) => match[1]),
  );
}

/** Command names hard-coded in the isolation script's own allow-list. */
function isolationAllowedCommands(): Set<string> {
  const source = readFileSync(ISOLATION_SCRIPT, 'utf8');
  const block = source.match(/const ALLOWED_COMMANDS = new Set\(\[([\s\S]*?)\]\)/);
  expect(block, 'ALLOWED_COMMANDS block must exist').not.toBeNull();
  return new Set([...block![1].matchAll(/'([a-z][a-z0-9_]*)'/g)].map((match) => match[1]));
}

describe('Tauri isolation IPC boundary', () => {
  it('forwards every generated-contract command with well-shaped arguments', () => {
    const hook = loadHook();
    const contractCommands = contractCommandNames();
    expect(contractCommands.size).toBeGreaterThan(0);
    for (const cmd of contractCommands) {
      expect(
        Object.prototype.hasOwnProperty.call(VALID_ARGUMENTS, cmd),
        `missing argument-shape fixture for generated command ${cmd}`,
      ).toBe(true);
      const message = invoke(cmd, VALID_ARGUMENTS[cmd]);
      expect(hook(message), `command ${cmd} should be forwarded`).toEqual(message);
    }
  });

  it('fails on contract-surface drift between bindings, fixtures and the isolation allow-list', () => {
    const contractCommands = contractCommandNames();
    expect([...Object.keys(VALID_ARGUMENTS)].sort()).toEqual([...contractCommands].sort());
    expect([...isolationAllowedCommands()].sort()).toEqual([...contractCommands].sort());
  });

  it('forwards the four event subcommands carrying a string event name', () => {
    const hook = loadHook();
    for (const subcommand of ['listen', 'unlisten', 'emit', 'once']) {
      const message = invoke(`plugin:event|${subcommand}`, { event: 'scan-progress' });
      expect(hook(message), `plugin:event|${subcommand} should be forwarded`).toEqual(message);
    }
  });

  it('drops event messages with an unknown subcommand or missing/non-string event', () => {
    const hook = loadHook();
    expect(() =>
      hook(invoke('plugin:event|emit_to', { event: 'scan-progress' })),
    ).toThrow(/invalid event message/);
    expect(() =>
      hook(invoke('plugin:event|bogus', { event: 'scan-progress' })),
    ).toThrow(/invalid event message/);
    expect(() => hook(invoke('plugin:event|listen', {}))).toThrow(/invalid event message/);
    expect(() => hook(invoke('plugin:event|once', { event: 42 }))).toThrow(
      /invalid event message/,
    );
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
