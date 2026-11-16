import type { ContractError } from '../client';
import { defaultSeedLogs } from './data';
import type { MockCtx } from './types';
import { defaultSettings } from './configOps';

export function resetMockCtx(ctx: MockCtx) {
  ctx.sessions.clear();
  ctx.analyzerSessions.clear();
  ctx.previews.clear();
  ctx.state.settingsGeneration = 1;
  ctx.state.scanSeq = 0;
  ctx.state.analyzerSeq = 0;
  ctx.state.logSeq = 0;
  ctx.previewSeq.value = 0;
  ctx.state.nowMs = Date.now();
  ctx.mutable.settingsState = defaultSettings();
  ctx.mutable.elevationStatus = { isElevated: false };
  ctx.mutable.relaunchResult = { launched: true };
  ctx.logs.length = 0;
  ctx.logs.push(...defaultSeedLogs({ nowMs: ctx.state.nowMs }));
}

export function buildTestApi(ctx: MockCtx) {
  return {
    advanceTimeMs(ms: number) {
      ctx.state.nowMs += ms;
    },
    invalidateSettings() {
      ctx.bumpSettings();
    },
    setElevationStatus(status: { isElevated: boolean }) {
      ctx.mutable.elevationStatus = { ...status };
    },
    setRelaunchResult(result: { launched: boolean } | { error: ContractError }) {
      ctx.mutable.relaunchResult = 'error' in result ? result : { ...result };
    },
    reset() {
      resetMockCtx(ctx);
    },
  };
}
