import type { BaseSummary, LogMeta, LogSummary } from './logParseTypes';
import { baseSummary, bytesToString, cleanOutcomeRecords, isScanSummaryRecord, parseLogObjects } from './logParseUtils';
import { formatBytes } from '../../utils/format';

function deletedText(count: number): string {
  let suffix = 's';
  if (count === 1) suffix = '';
  return `${count} item${suffix}`;
}
function formatBytesString(value: string): string { return formatBytes({ bytes: value }); }
function cleanTitle(args: { freed: string | null; count: number }): string {
  if (args.freed) {
    if (args.count > 0) return `Cleaned ${args.freed} \u00b7 ${deletedText(args.count)}`;
    return `Cleaned ${args.freed}`;
  }
  if (args.count > 0) return `Cleaned ${deletedText(args.count)}`;
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
function scanTitle(o: Record<string, unknown>): string {
  const files = scanFilesCount(o);
  const reclaim = scanReclaimable(o);
  if (reclaim && files != null) {
    let word = 'files';
    if (files === 1) word = 'file';
    return `Scan Completed \u00b7 ${reclaim} \u00b7 ${files} ${word}`;
  }
  if (reclaim) return `Scan Completed \u00b7 ${reclaim}`;
  const cats = o.categories_count ?? o.categoriesCount;
  if (cats != null) {
    const n = Number(cats);
    if (Number.isFinite(n) && n > 0) return `Scan Completed \u00b7 ${n} ${n === 1 ? 'category' : 'categories'}`;
  }
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
