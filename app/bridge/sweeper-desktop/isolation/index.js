// Tauri isolation application script.
// Enforces a command-name allow-list and basic argument-shape checks on IPC
// messages crossing from the untrusted webview into Tauri core. This is
// defence in depth only: the Rust-side command validation remains authoritative.

'use strict';

// Mirrors the `commands.allow` list in capabilities/default.json.
const ALLOWED_COMMANDS = new Set([
  'start_scan',
  'get_scan_results',
  'preview_clean',
  'execute_clean',
  'start_analyzer',
  'cancel_analyzer',
  'explain_item',
  'get_settings',
  'update_settings',
  'validate_custom_path',
  'list_logs',
  'read_log',
  'get_elevation_status',
  'relaunch_elevated',
  'resolve_close',
  'notify_close_listener_ready',
  'reset_pending_close',
]);

// Tauri's event-plugin messages carry progress events emitted by the backend.
// They are gated by Tauri's own capability system; forwarding them here keeps
// scan/clean/analyzer progress subscriptions working. Only the subcommands the
// frontend actually uses are forwarded; everything else is blocked here.
const INTERNAL_COMMAND_PREFIX = 'plugin:event|';
const ALLOWED_EVENT_SUBCOMMANDS = new Set(['listen', 'unlisten', 'emit', 'once']);

function reject(reason) {
  throw new Error(`Blocked IPC message: ${reason}`);
}

function isRecord(value) {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isString(value) {
  return typeof value === 'string';
}

function isBoolean(value) {
  return typeof value === 'boolean';
}

function isStringArray(value) {
  return Array.isArray(value) && value.every(isString);
}

function isOptional(value, predicate) {
  return value === undefined || value === null || predicate(value);
}

// Basic per-command argument shapes derived from the contract bindings.
const ARGUMENT_SHAPES = {
  start_scan: { tiers: (v) => isOptional(v, isStringArray) },
  get_scan_results: { sessionId: isString },
  preview_clean: {
    sessionId: isString,
    itemIds: isStringArray,
    confirmations: (v) => isOptional(v, isRecord),
  },
  execute_clean: { previewToken: isString },
  start_analyzer: { root: (v) => isOptional(v, isString) },
  cancel_analyzer: { sessionId: isString },
  explain_item: { itemId: isString },
  get_settings: {},
  update_settings: { settings: isRecord },
  validate_custom_path: { path: isString },
  list_logs: {},
  read_log: { logId: isString },
  get_elevation_status: {},
  relaunch_elevated: {},
  resolve_close: { approved: isBoolean, requestId: isString },
  notify_close_listener_ready: {},
  reset_pending_close: {},
};

function hasValidArguments(cmd, payload) {
  const shape = ARGUMENT_SHAPES[cmd];
  if (!shape) return false;
  const args = isRecord(payload) ? payload : {};
  return Object.entries(shape).every(([key, predicate]) => predicate(args[key]));
}

// Every supported event-plugin subcommand carries a string `event` name (the
// @tauri-apps/api event module always supplies it, including for unlisten).
// Defence in depth only: Rust validates the event name authoritatively.
function isAllowedEventMessage(cmd, payload) {
  const subcommand = cmd.slice(INTERNAL_COMMAND_PREFIX.length);
  return (
    ALLOWED_EVENT_SUBCOMMANDS.has(subcommand) &&
    isRecord(payload) &&
    isString(payload.event)
  );
}

window.__TAURI_ISOLATION_HOOK__ = (message) => {
  if (!isRecord(message)) {
    reject('malformed message');
  }
  const cmd = message.cmd;
  if (typeof cmd !== 'string') {
    reject('missing command name');
  }
  if (cmd.startsWith(INTERNAL_COMMAND_PREFIX)) {
    if (!isAllowedEventMessage(cmd, message.payload)) {
      reject(`invalid event message for '${cmd}'`);
    }
    return message;
  }
  if (!ALLOWED_COMMANDS.has(cmd)) {
    reject(`unknown command '${cmd}'`);
  }
  if (!hasValidArguments(cmd, message.payload)) {
    reject(`invalid arguments for '${cmd}'`);
  }
  return message;
};
