import { err } from './helpers';
import type { MockCtx } from './types';

/**
 * A Clean must never start while a Scan is running (#95, #143). This is the
 * mock counterpart of the backend `reject_clean_while_scan_running` preflight,
 * shared by Preview (token issuance) and Execute (token consumption) so a
 * rejection issues no token, consumes none, and deletes nothing.
 *
 * Only mock scan sessions with a live worker count: a completed, cancelled, or
 * stale session remains in `ctx.sessions` for result lookup but is no longer
 * `running`, exactly like the backend's removed `active_scans` registration.
 */
export function rejectCleanWhileScanRunning(ctx: MockCtx): void {
  for (const session of ctx.sessions.values()) {
    if (session.running) {
      err({ code: 'internal', reason: 'Clean rejected: a Scan is already running' });
    }
  }
}
