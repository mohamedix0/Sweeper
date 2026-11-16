import type { RiskDto, Settings } from '../client';
import { err } from './helpers';
import type { MockCtx } from './types';

export function defaultSettings(): Settings {
  return {
    enableTier1: true,
    enableTier2: true,
    enableTier3: false,
    enableElevationTargets: false,
    minAgeHours: 24,
    allowDeleteUnknown: false,
    customPaths: [],
    staleProjectDays: 30,
  };
}

function cloneSettings(settings: Settings): Settings {
  return { ...settings, customPaths: [...settings.customPaths] };
}

function validatePath(input: { path: string }) {
  const trimmed = input.path.trim();
  if (!trimmed) return { path: trimmed, allowed: false, reason: 'Path is required.' };
  if (!/^[A-Za-z]:\\/.test(trimmed)) return { path: trimmed, allowed: false, reason: 'Path must be absolute.' };
  const lower = trimmed.toLowerCase();
  if (lower.startsWith('c:\\windows')) return { path: trimmed, allowed: false, reason: 'Protected Path (no fallback to parent).' };
  if (lower.includes('\\missing')) return { path: trimmed, allowed: false, reason: 'Path does not exist (no fallback to parent).' };
  if ((trimmed.match(/\\/g) || []).length < 2) return { path: trimmed, allowed: false, reason: 'Path must have depth >= 3 segments.' };
  return { path: trimmed, allowed: true, reason: 'ok' };
}

function normalizeUpdatedSettings(settings: Settings): Settings {
  return {
    ...settings,
    enableElevationTargets: settings.enableElevationTargets ?? false,
    customPaths: [...(settings.customPaths ?? [])],
  };
}

type Explanation = { itemId: string; title: string; description: string; risk: RiskDto };

function findExplanation(input: { ctx: MockCtx; itemId: string }): Explanation | null {
  for (const session of input.ctx.sessions.values()) {
    const item = session.itemsById.get(input.itemId);
    if (item) return { itemId: input.itemId, title: item.targetName, description: item.description, risk: item.risk };
  }
  for (const session of input.ctx.analyzerSessions.values()) {
    const node = session.nodesById.get(input.itemId);
    if (node) return { itemId: input.itemId, title: node.name || node.path, description: node.description, risk: node.risk };
  }
  return null;
}

function explainItem(input: { ctx: MockCtx; itemId: string }) {
  const found = findExplanation(input);
  if (found) return found;
  err({ code: 'notFound', reason: 'Item not found.' });
}

function findLog(input: { ctx: MockCtx; logId: string }) {
  return input.ctx.logs.find((entry) => entry.id === input.logId) ?? null;
}

export function buildConfigApi(ctx: MockCtx) {
  return {
    async getSettings() {
      return cloneSettings(ctx.mutable.settingsState);
    },
    async updateSettings(settings: Settings) {
      ctx.mutable.settingsState = normalizeUpdatedSettings(settings);
      ctx.bumpSettings();
      return cloneSettings(ctx.mutable.settingsState);
    },
    async getElevationStatus() {
      return { ...ctx.mutable.elevationStatus };
    },
    async relaunchElevated() {
      if ('error' in ctx.mutable.relaunchResult) throw ctx.mutable.relaunchResult.error;
      return { ...ctx.mutable.relaunchResult };
    },
    async validateCustomPath(path: string) {
      return validatePath({ path });
    },
    async listLogs() {
      return ctx.logs.map((log) => ({
        id: log.id,
        name: log.name,
        modifiedUnixSecs: log.modifiedUnixSecs,
        sizeBytes: log.sizeBytes,
      }));
    },
    async readLog(logId: string) {
      const log = findLog({ ctx, logId });
      if (!log) err({ code: 'notFound', reason: 'Log not found.' });
      return { id: log.id, content: log.content };
    },
    async explainItem(itemId: string) {
      return explainItem({ ctx, itemId });
    },
  };
}
