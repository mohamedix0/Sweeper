import type { AuditRow, Flags } from './logParseTypes';
import { parseLogObjects, detectCleanScan, rowSizeText } from './logParseUtils';
import { normalizeOutcome } from './logParseOutcome';

function isScanSummaryEvent(ev: string, o: Record<string, unknown>): boolean { if (ev !== 'scan') return false; if (o.categoriesCount != null) return true; return o.totalBytes != null; }
function rowRecordIgnored(rec: string): boolean { if (rec === 'outcome') return true; return rec === 'intent'; }
function rowHasData(o: Record<string, unknown>): boolean {
  if (o.category) return true; if (o.path) return true; if (o.target) return true;
  if (o.size) return true; if (o.bytes) return true; return Boolean(o.status);
}
function isItemRow(o: Record<string, unknown>): boolean {
  const rec = String(o.record ?? '');
  if (rowRecordIgnored(rec)) return false;
  const ev = String(o.event ?? o.action ?? '').toLowerCase();
  if (isScanSummaryEvent(ev, o)) return false;
  return rowHasData(o);
}
function skippedParentLabel(rawCategory: string, location: string): string {
  if (rawCategory !== 'Custom Path (Unknown)') return rawCategory;
  const parts = location.split(/[\\/]/).filter(Boolean);
  const name = parts[parts.length - 2];
  if (name) return `Custom Path · ${name}`;
  return 'Custom Path · CustomCleanup';
}
function resolveSkippedFileRow(rawCategory: string, location: string, tone: AuditRow['outcomeTone']) {
  if (tone !== 'skipped') return null;
  if (!location) return null;
  if (location === '—') return null;
  const parts = location.split(/[\\/]/).filter(Boolean);
  let category = parts[parts.length - 1];
  if (!category) category = rawCategory;
  return { category, sublabel: skippedParentLabel(rawCategory, location) };
}
function outcomeDefault(flags: Flags): 'cleaned' | 'eligible' | 'neutral' { if (flags.isClean) return 'cleaned'; if (flags.isScan) return 'eligible'; return 'neutral'; }
function outcomeRaw(o: Record<string, unknown>, flags: Flags): unknown {
  if (o.status != null) return o.status;
  if (flags.isClean) return 'Cleaned';
  if (flags.isScan) return 'Eligible';
  return 'Recorded';
}
function itemOutcome(o: Record<string, unknown>, flags: Flags) { return normalizeOutcome({ raw: outcomeRaw(o, flags), fallback: outcomeDefault(flags) }); }
function itemCategory(o: Record<string, unknown>, flags: Flags): string {
  if (o.category != null) return String(o.category);
  if (o.target != null) return String(o.target);
  if (o.action != null) return String(o.action);
  if (flags.isScan) return 'Scan';
  return 'Cleanup';
}
function itemKind(o: Record<string, unknown>): 'Category' | 'File' { if (o.kind === 'Category') return 'Category'; if (o.kind === 'File') return 'File'; return 'File'; }
function itemSublabel(kind: 'Category' | 'File', skipped: { sublabel: string } | null, tone: AuditRow['outcomeTone']) {
  if (kind === 'Category') return undefined;
  if (skipped) return skipped.sublabel;
  if (tone === 'skipped') return 'Skipped file';
  return undefined;
}
function rowType(tone: AuditRow['outcomeTone']): string { if (tone === 'skipped') return 'Skipped file'; return 'Category'; }
function skippedForRow(kind: 'Category' | 'File', rawCategory: string, location: string, tone: AuditRow['outcomeTone']) {
  if (kind !== 'File') return null;
  return resolveSkippedFileRow(rawCategory, location, tone);
}
function normalizeItemRow(o: Record<string, unknown>, flags: Flags): AuditRow {
  const rawCategory = itemCategory(o, flags);
  const location = String(o.path ?? o.details ?? o.location ?? '—');
  const size = rowSizeText(o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes);
  const outcome = itemOutcome(o, flags);
  const kind = itemKind(o);
  const skipped = skippedForRow(kind, rawCategory, location, outcome.tone);
  let category = rawCategory;
  if (skipped) category = skipped.category;
  return { type: rowType(outcome.tone), category, location, size, outcome: outcome.label, outcomeTone: outcome.tone, kind, sublabel: itemSublabel(kind, skipped, outcome.tone) };
}
function fallbackCleanLocation(outcome: Record<string, unknown> | null): string {
  const deleted = outcome?.deletedCount;
  if (deleted == null) return 'Completed cleanup';
  const n = Number(deleted);
  let label = 'items';
  if (n === 1) label = 'item';
  return `${n} ${label} removed`;
}
function fallbackCleanSize(outcome: Record<string, unknown> | null): string { if (outcome?.bytesFreed == null) return '\u2014'; return rowSizeText(outcome.bytesFreed); }
function fallbackCleanCategory(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): string {
  if (outcome?.category != null) return String(outcome.category);
  if (intent?.category != null) return String(intent.category);
  return 'Cleanup';
}
function fallbackCleanRow(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): AuditRow {
  return { type: 'Category', category: fallbackCleanCategory(outcome, intent), location: fallbackCleanLocation(outcome), size: fallbackCleanSize(outcome), outcome: 'Cleaned', outcomeTone: 'cleaned', kind: 'Category', sublabel: 'Category' };
}
function pluralLabel(args: { value: unknown; dash: string; singular: string; plural: string }): string {
  if (args.value == null) return args.dash;
  const n = Number(args.value);
  let word = args.plural;
  if (n === 1) word = args.singular;
  return `${args.value} ${word}`;
}
function scanCategoryText(cats: unknown, dash: string): string { return pluralLabel({ value: cats, dash, singular: 'category', plural: 'categories' }); }
function fallbackScanLocation(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): string {
  const cats = outcome?.categoriesCount ?? intent?.categoriesCount;
  const items = outcome?.items ?? intent?.items;
  const dash = '\u2014';
  const catText = scanCategoryText(cats, dash);
  if (items == null) return catText;
  return `${catText} \u00b7 ${items} items`;
}
function fallbackScanSize(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): string {
  const total = outcome?.totalBytes ?? intent?.totalBytes;
  if (total == null) return '\u2014';
  return rowSizeText(total);
}
function fallbackScanRow(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): AuditRow {
  return { type: 'Category', category: 'System Scan', location: fallbackScanLocation(outcome, intent), size: fallbackScanSize(outcome, intent), outcome: 'Safe', outcomeTone: 'eligible', kind: 'Category', sublabel: 'Category' };
}
function fallbackRows(flags: ReturnType<typeof detectCleanScan>): AuditRow[] {
  if (flags.isClean) return [fallbackCleanRow(flags.outcome, flags.intent)];
  if (flags.isScan) return [fallbackScanRow(flags.outcome, flags.intent)];
  return [{ type: 'Category', category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral', kind: 'Category', sublabel: 'Category' }];
}
export function parseAuditRows(content: string): AuditRow[] {
  const objs = parseLogObjects(content);
  if (objs.length === 0) return [];
  const flags = detectCleanScan(objs);
  const itemRows = objs.filter(isItemRow);
  if (itemRows.length > 0) return itemRows.map((o) => normalizeItemRow(o, flags));
  return fallbackRows(flags);
}
