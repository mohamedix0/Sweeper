import type { BaseSummary, LogMeta, LogSummary } from './logParseTypes';
import { baseSummary } from './logParseUtils';
import { formatBytes } from '../../utils/format';

function deletedText(outcome: Record<string, unknown>): string | null {
  const deleted = outcome.deletedCount ?? outcome.deleted;
  if (deleted == null) return null;
  let suffix = 's';
  if (String(deleted) === '1') suffix = '';
  return `${String(deleted)} item${suffix}`;
}
function freedText(outcome: Record<string, unknown>): string | null {
  const value = outcome.bytesFreed;
  if (typeof value === 'string') return formatBytes({ bytes: value });
  if (typeof value === 'number') return formatBytes({ bytes: String(value) });
  return null;
}
function cleanTitleWithFreed(args: { freed: string; deletedStr: string | null }): string {
  if (args.deletedStr) return `Cleaned ${args.freed} \u00b7 ${args.deletedStr}`;
  return `Cleaned ${args.freed}`;
}
function cleanTitle(outcome: Record<string, unknown>): string {
  const deletedStr = deletedText(outcome);
  const freed = freedText(outcome);
  if (freed) return cleanTitleWithFreed({ freed, deletedStr });
  if (deletedStr) return `Cleaned ${deletedStr}`;
  return 'Clean Operation';
}
function summarizeCleanOutcome(outcome: Record<string, unknown>, base: BaseSummary): LogSummary { return { title: cleanTitle(outcome), subtitle: 'Completed', ...base, kind: 'clean' }; }
function scanDetail(raw: unknown): string {
  if (raw == null) return '';
  const count = Number(raw);
  if (count <= 0) return '';
  let word = 'categories';
  if (count === 1) word = 'category';
  return ` · ${count} ${word}`;
}
function scanTitle(outcome: Record<string, unknown>): string { return `System Scan${scanDetail(outcome.categoriesCount)}`; }
function summarizeScanOutcome(outcome: Record<string, unknown>, base: BaseSummary): LogSummary { return { title: scanTitle(outcome), subtitle: 'Completed', ...base, kind: 'scan' }; }
function isScanSummaryEventRecord(outcome: Record<string, unknown>): boolean { if (String(outcome.event) === 'scan') return true; return outcome.action === 'scan'; }
function isCleanOutcome(outcome: Record<string, unknown>): boolean {
  if (isScanSummaryEventRecord(outcome)) return false;
  if (outcome.deletedCount != null) return true;
  if (outcome.bytesFreed != null) return true;
  if (String(outcome.record) !== 'outcome') return false;
  if (outcome.event === 'clean') return true;
  return outcome.action === 'clean';
}
function hasScanOutcomeData(outcome: Record<string, unknown>): boolean { if (outcome.categoriesCount != null) return true; return outcome.totalBytes != null; }
function isScanOutcome(outcome: Record<string, unknown>): boolean {
  if (String(outcome.event) === 'scan') return true;
  if (outcome.action === 'scan') return true;
  if (String(outcome.record) !== 'outcome') return false;
  if (!hasScanOutcomeData(outcome)) return false;
  return outcome.deletedCount == null;
}
function isSummaryRecord(o: Record<string, unknown>): boolean {
  if (o.record === 'outcome') return true;
  if (o.event === 'clean') return true;
  if (o.event === 'scan') return true;
  if (o.action === 'scan') return true;
  return o.action === 'clean';
}
function findSummaryOutcome(lines: Record<string, unknown>[]) { return [...lines].reverse().find((o) => isSummaryRecord(o)); }
function summaryFromOutcome(outcome: Record<string, unknown> | undefined, base: BaseSummary): LogSummary | null {
  if (!outcome) return null;
  if (isCleanOutcome(outcome)) return summarizeCleanOutcome(outcome, base);
  if (isScanOutcome(outcome)) return summarizeScanOutcome(outcome, base);
  return null;
}
function parseLines(content: string): Record<string, unknown>[] { return content.split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l) as Record<string, unknown>); }
function summarizeFromContent(content: string | null, base: BaseSummary): LogSummary | null {
  if (!content) return null;
  try { return summaryFromOutcome(findSummaryOutcome(parseLines(content)), base); } catch { return null; }
}
function fallbackSummary(meta: LogMeta, base: BaseSummary): LogSummary {
  const lower = meta.name.toLowerCase();
  if (lower.includes('clean')) return { title: 'Clean Operation', subtitle: 'Completed', ...base, kind: 'clean' };
  if (lower.includes('scan')) return { title: 'System Scan', subtitle: 'Completed', ...base, kind: 'scan' };
  return { title: meta.name.replace(/\.jsonl$/i, ''), subtitle: 'Completed', ...base, kind: 'all' };
}
export function summarizeLog(meta: LogMeta, content: string | null): LogSummary {
  const base = baseSummary(meta);
  const summary = summarizeFromContent(content, base);
  if (summary) return summary;
  return fallbackSummary(meta, base);
}
