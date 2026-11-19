import type { BaseSummary, LogMeta, LogSummary } from './logParseTypes';
import { baseSummary, bytesToString, cleanOutcomeRecords, isScanSummaryRecord, parseLogObjects } from './logParseUtils';
import { formatBytes, pluralize } from '../../utils/format';

function formatBytesString(value: string): string { return formatBytes({ bytes: value }); }
function cleanTitle(args: { freed: string | null; count: number }): string {
  const items = pluralize({ count: args.count, singular: 'item', plural: 'items' });
  if (args.freed) {
    if (args.count > 0) return `Cleaned ${args.freed} \u00b7 ${items}`;
    return `Cleaned ${args.freed}`;
  }
  if (args.count > 0) return `Cleaned ${items}`;
  return 'Clean Operation';
}
function summarizeClean(objs: Record<string, unknown>[], base: BaseSummary): LogSummary {
  const outcomes = cleanOutcomeRecords(objs);
  let freedNum = 0n;
  let deletedCount = 0;
  for (const o of outcomes) {
    const status = String(o.status ?? '').toLowerCase();
    if (status !== 'deleted') continue;
    deletedCount += 1;
    const b = bytesToString(o.bytes ?? o.size ?? 0) ?? '0';
    try { freedNum += BigInt(b); } catch { /* ignore */ }
  }
  const freed = deletedCount > 0 || freedNum > 0n ? formatBytesString(freedNum.toString()) : null;
  return { title: cleanTitle({ freed, count: deletedCount }), subtitle: 'Completed', ...base, kind: 'clean' };
}

function scanReclaimable(o: Record<string, unknown>): string | null {
  const v = o.total_reclaimable_bytes ?? o.eligible_bytes ?? o.totalBytes;
  const s = bytesToString(v);
  if (s == null) return null;
  return formatBytesString(s);
}
function scanFilesCount(o: Record<string, unknown>): number | null {
  const v = o.total_eligible_files ?? o.eligible_count;
  const s = bytesToString(v);
  if (s == null) return null;
  const n = Number(s);
  if (!Number.isFinite(n)) return null;
  return n;
}
function positiveCategoryCount(o: Record<string, unknown>): number | null {
  const cats = o.categories_count ?? o.categoriesCount;
  if (cats == null) return null;
  const n = Number(cats);
  if (!Number.isFinite(n) || n <= 0) return null;
  return n;
}

function scanTitleWithCounts(reclaim: string | null, files: number | null): string | null {
  if (reclaim && files != null) {
    return `Scan Completed \u00b7 ${reclaim} \u00b7 ${pluralize({ count: files, singular: 'file', plural: 'files' })}`;
  }
  if (reclaim) return `Scan Completed \u00b7 ${reclaim}`;
  return null;
}

function scanTitle(o: Record<string, unknown>): string {
  const files = scanFilesCount(o);
  const reclaim = scanReclaimable(o);
  const withCounts = scanTitleWithCounts(reclaim, files);
  if (withCounts) return withCounts;
  const n = positiveCategoryCount(o);
  if (n != null) return `Scan Completed \u00b7 ${pluralize({ count: n, singular: 'category', plural: 'categories' })}`;
  return 'Scan Completed';
}
function summarizeScan(objs: Record<string, unknown>[], base: BaseSummary): LogSummary {
  const scan = [...objs].reverse().find(isScanSummaryRecord) ?? objs[objs.length - 1] ?? {};
  return { title: scanTitle(scan), subtitle: 'Completed', ...base, kind: 'scan' };
}

function isLegacyCleanSummary(o: Record<string, unknown>): boolean {
  if (isScanSummaryRecord(o)) return false;
  if (o.deletedCount != null) return true;
  return o.bytesFreed != null;
}
function legacyCleanTitle(outcome: Record<string, unknown>): string {
  const deleted = outcome.deletedCount ?? outcome.deleted;
  const freed = bytesToString(outcome.bytesFreed);
  const deletedCount = deleted == null ? 0 : Number(deleted);
  const freedStr = freed ? formatBytesString(freed) : null;
  return cleanTitle({ freed: freedStr, count: Number.isFinite(deletedCount) ? deletedCount : 0 });
}

function fallbackSummary(meta: LogMeta, base: BaseSummary): LogSummary {
  const lower = meta.name.toLowerCase();
  if (lower.includes('scan')) return { title: 'Scan Completed', subtitle: 'Completed', ...base, kind: 'scan' };
  if (lower.includes('clean')) return { title: 'Clean Operation', subtitle: 'Completed', ...base, kind: 'clean' };
  return { title: meta.name.replace(/\.jsonl$/i, ''), subtitle: 'Completed', ...base, kind: 'all' };
}

export function summarizeLog(meta: LogMeta, content: string | null): LogSummary {
  const base = baseSummary(meta);
  if (!content) return fallbackSummary(meta, base);
  try {
    const objs = parseLogObjects(content);
    if (objs.some(isScanSummaryRecord) || meta.name.toLowerCase().startsWith('scan-')) return summarizeScan(objs, base);
    if (cleanOutcomeRecords(objs).length > 0) return summarizeClean(objs, base);
    const legacy = [...objs].reverse().find(isLegacyCleanSummary);
    if (legacy) return { title: legacyCleanTitle(legacy), subtitle: 'Completed', ...base, kind: 'clean' };
    return fallbackSummary(meta, base);
  } catch {
    return fallbackSummary(meta, base);
  }
}
