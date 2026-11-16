import type { ScanResults } from './api/client';

export function makeScanResults(input: { sessionId: string; targets: unknown[]; minAgeHours?: number }): ScanResults {
  return { sessionId: input.sessionId, startedUnixSecs: '1710000000', minAgeHours: input.minAgeHours ?? 24, targets: input.targets } as ScanResults;
}

export function explainOk(input: { itemId: string; title?: string; risk?: 'Safe' | 'Caution' | 'Unknown' }) {
  return { itemId: input.itemId, title: input.title ?? input.itemId, description: 'desc', risk: input.risk ?? ('Safe' as const) };
}