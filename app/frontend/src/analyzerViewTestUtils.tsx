import type { StartAnalyzerResult } from './api/client';

export function makeAnalyzerResult(input: { sessionId: string; root: unknown; rootPath?: string; totalBytes?: string; totalItems?: string }): StartAnalyzerResult {
  return {
    sessionId: input.sessionId,
    rootPath: input.rootPath ?? 'C:\\',
    totalBytes: input.totalBytes ?? '0',
    totalItems: input.totalItems ?? '0',
    scannedDirs: '1',
    scannedFiles: '1',
    cancelled: false,
    warnings: [],
    root: input.root as any,
  };
}

export function explainOk(input: { itemId: string; title?: string; risk?: 'Safe' | 'Caution' | 'Unknown' }) {
  return { itemId: input.itemId, title: input.title ?? input.itemId, description: 'desc', risk: input.risk ?? ('Safe' as const) };
}