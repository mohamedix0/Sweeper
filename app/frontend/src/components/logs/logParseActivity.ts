import type { ActivitySummary, AuditRow, Flags } from './logParseTypes';
import { detectCleanScan, modeFor, outcomeBytes, parseLogObjects } from './logParseUtils';
import { parseAuditRows } from './logParseRows';

function primaryMetric(flags: Flags, outcome: Record<string, unknown> | undefined): Pick<ActivitySummary, 'primaryLabel' | 'primaryValue'> {
  if (flags.isClean) return { primaryLabel: 'Space Reclaimed', primaryValue: outcomeBytes(outcome, 'bytesFreed') };
  if (flags.isScan) return { primaryLabel: 'Space Found', primaryValue: outcomeBytes(outcome, 'totalBytes') };
  return { primaryLabel: 'Space Reclaimed', primaryValue: '—' };
}
function rowParentLabel(r: AuditRow): string { if (r.sublabel) return r.sublabel; if (r.category) return r.category; return ''; }
function countCategories(skippedFileRows: AuditRow[]): number {
  const parents = skippedFileRows.map((r) => rowParentLabel(r).split('·')[0].trim()).filter(Boolean);
  const unique = new Set(parents).size;
  if (unique > 0) return unique;
  return skippedFileRows.length;
}
function cleanedCategoryCount(rows: AuditRow[]): number { return rows.filter((r) => r.kind === 'Category').filter((r) => r.outcomeTone === 'cleaned').length; }
function eligibleCategoryCount(rows: AuditRow[]): number { return rows.filter((r) => r.kind === 'Category').filter((r) => r.outcomeTone === 'eligible').length; }
function pickCount(flags: Flags, rows: AuditRow[]): number { if (flags.isClean) return cleanedCategoryCount(rows); return eligibleCategoryCount(rows); }
function fallbackCategoryCount(outcome?: Record<string, unknown>, intent?: Record<string, unknown> | null): number {
  if (outcome?.categoriesCount != null) return Number(outcome.categoriesCount);
  if (intent?.categoriesCount != null) return Number(intent.categoriesCount);
  return 0;
}
function eligibleCategories(args: { flags: Flags; rows: AuditRow[]; outcome?: Record<string, unknown>; intent?: Record<string, unknown> | null }): number {
  const count = pickCount(args.flags, args.rows);
  if (count) return count;
  return fallbackCategoryCount(args.outcome, args.intent);
}
function cleanedFileCount(rows: AuditRow[]): number { return rows.filter((r) => r.kind === 'File').filter((r) => r.outcomeTone === 'cleaned').length; }
function eligibleFileRows(rows: AuditRow[]): number { return rows.filter((r) => r.kind === 'File').filter((r) => r.outcomeTone === 'eligible').length; }
function skippedFileRowsCount(rows: AuditRow[]): number { return rows.filter((r) => r.kind === 'File').filter((r) => r.outcomeTone === 'skipped').length; }
function outcomeItems(outcome?: Record<string, unknown>): number { if (outcome?.items == null) return 0; return Number(outcome.items); }
function scanEligibleFiles(rows: AuditRow[], outcome?: Record<string, unknown>): number {
  const eligible = eligibleFileRows(rows);
  if (eligible) return eligible;
  const value = outcomeItems(outcome) - skippedFileRowsCount(rows);
  return Math.max(0, value);
}
function cleanEligibleFiles(rows: AuditRow[], outcome?: Record<string, unknown>): number {
  if (outcome?.deletedCount != null) return Number(outcome.deletedCount);
  return cleanedFileCount(rows);
}
function eligibleFiles(flags: Flags, rows: AuditRow[], outcome?: Record<string, unknown>): number {
  if (flags.isClean) return cleanEligibleFiles(rows, outcome);
  return scanEligibleFiles(rows, outcome);
}
function pluralize(args: { n: number; one: string; many: string }): string { let word = args.many; if (args.n === 1) word = args.one; return `${args.n} ${word}`; }
function summaryCounts(args: { flags: Flags; rows: AuditRow[]; outcome?: Record<string, unknown>; intent?: Record<string, unknown> | null }) {
  const files = eligibleFiles(args.flags, args.rows, args.outcome);
  const cats = eligibleCategories(args);
  const skippedRows = args.rows.filter((r) => r.kind === 'File').filter((r) => r.outcomeTone === 'skipped');
  return { eligible: `${pluralize({ n: files, one: 'file', many: 'files' })} · ${pluralize({ n: cats, one: 'category', many: 'categories' })}`, skipped: `${pluralize({ n: skippedRows.length, one: 'file', many: 'files' })} · ${pluralize({ n: countCategories(skippedRows), one: 'category', many: 'categories' })}` };
}
function emptyActivity(): ActivitySummary { return { primaryLabel: 'Space Reclaimed', primaryValue: '—', eligible: '—', skipped: '—', outcome: '—', mode: null }; }
function statusOutcomeText(status: unknown): string { if (status === 'completed') return 'Completed'; return String(status); }
function buildActivity(content: string): ActivitySummary {
  const objs = parseLogObjects(content);
  const flags = detectCleanScan(objs);
  const primary = primaryMetric(flags, flags.outcome ?? undefined);
  const rows = parseAuditRows(content);
  const counts = summaryCounts({ flags, rows, outcome: flags.outcome ?? undefined, intent: flags.intent });
  let status: unknown = 'completed';
  if (flags.outcome) status = flags.outcome.status ?? 'completed';
  return { ...primary, ...counts, outcome: statusOutcomeText(status), mode: modeFor(flags) };
}
export function deriveActivitySummary(content: string | null): ActivitySummary {
  if (!content) return emptyActivity();
  try { return buildActivity(content); } catch { return { ...emptyActivity(), outcome: 'Recorded' }; }
}
