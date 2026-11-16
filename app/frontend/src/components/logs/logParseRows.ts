import type { AuditRow, Flags } from './logParseTypes';
import { bytesToString, cleanOutcomeRecords, detectCleanScan, isJournalPerItemOutcome, isScanSummaryRecord, parseLogObjects, rowSizeText } from './logParseUtils';
import { normalizeOutcome } from './logParseOutcome';

function isScanSummaryEvent(ev: string, o: Record<string, unknown>): boolean {
  if (ev !== 'scan') return false;
  if (o.categoriesCount != null) return true;
  if (o.categories_count != null) return true;
  if (o.totalBytes != null) return true;
  return o.total_reclaimable_bytes != null || Array.isArray(o.targets);
}
function rowRecordIgnored(rec: string, o: Record<string, unknown>): boolean {
  if (isJournalPerItemOutcome(o)) return false;
  if (rec === 'outcome') return true;
  return rec === 'intent';
}
function rowHasData(o: Record<string, unknown>): boolean {
  if (o.category) return true; if (o.path) return true; if (o.target) return true;
  if (o.size) return true; if (o.bytes) return true; if (o.target_id) return true; return Boolean(o.status);
}
function isItemRow(o: Record<string, unknown>): boolean {
  const rec = String(o.record ?? '');
  if (rowRecordIgnored(rec, o)) return false;
  const ev = String(o.event ?? o.action ?? '').toLowerCase();
  if (isScanSummaryEvent(ev, o) || isScanSummaryRecord(o)) return false;
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
  if (!location || location === '—') return null;
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
  if (o.target_name != null) return String(o.target_name);
  if (o.target_id != null) return String(o.target_id);
  if (o.action != null) return String(o.action);
  if (flags.isScan) return 'Scan';
  return 'Cleanup';
}
function itemKind(o: Record<string, unknown>): 'Category' | 'File' {
  if (o.kind === 'Category') return 'Category';
  if (o.kind === 'File') return 'File';
  if (isJournalPerItemOutcome(o)) return 'File';
  return 'File';
}
function itemSublabel(kind: 'Category' | 'File', skipped: { sublabel: string } | null, tone: AuditRow['outcomeTone'], targetId?: unknown) {
  if (kind === 'Category') return undefined;
  if (skipped) return skipped.sublabel;
  if (tone === 'skipped') {
    if (targetId) return `Skipped file · ${String(targetId)}`;
    return 'Skipped file';
  }
  if (targetId) return String(targetId);
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
  const size = rowSizeText(o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes ?? o.eligible_bytes);
  const outcome = itemOutcome(o, flags);
  const kind = itemKind(o);
  const skipped = skippedForRow(kind, rawCategory, location, outcome.tone);
  let category = rawCategory;
  if (skipped) category = skipped.category;
  return { type: rowType(outcome.tone), category, location, size, outcome: outcome.label, outcomeTone: outcome.tone, kind, sublabel: itemSublabel(kind, skipped, outcome.tone, o.target_id) };
}

function cleanRowsFromJournal(objs: Record<string, unknown>[]): AuditRow[] {
  const outcomes = cleanOutcomeRecords(objs);
  if (outcomes.length === 0) return [];
  const rows: AuditRow[] = [];
  const cleanedByTarget = new Map<string, { bytes: bigint; count: number }>();
  for (const o of outcomes) {
    const status = String(o.status ?? '').toLowerCase();
    const targetId = String(o.target_id ?? 'Cleanup');
    if (status === 'deleted') {
      const cur = cleanedByTarget.get(targetId) ?? { bytes: 0n, count: 0 };
      const b = bytesToString(o.bytes ?? 0) ?? '0';
      try { cur.bytes += BigInt(b); } catch { /* ignore */ }
      cur.count += 1;
      cleanedByTarget.set(targetId, cur);
    }
  }
  for (const [targetId, agg] of cleanedByTarget.entries()) {
    rows.push({
      type: 'Category',
      category: targetId,
      location: `${agg.count} ${agg.count === 1 ? 'item' : 'items'} cleaned`,
      size: rowSizeText(agg.bytes.toString()),
      outcome: 'Cleaned',
      outcomeTone: 'cleaned',
      kind: 'Category',
      sublabel: 'Category',
    });
  }
  for (const o of outcomes) {
    const status = String(o.status ?? '').toLowerCase();
    if (status !== 'skipped' && status !== 'aborted') continue;
    const reason = String(o.reason ?? '');
    rows.push({
      type: 'Skipped file',
      category: String(o.path ?? o.target_id ?? 'Item').split(/[\\/]/).filter(Boolean).pop() ?? 'Item',
      location: String(o.path ?? '—'),
      size: rowSizeText(o.bytes ?? o.size ?? 0),
      outcome: reason ? `Skipped · ${reason}` : 'Skipped',
      outcomeTone: 'skipped',
      kind: 'File',
      sublabel: o.target_id ? `Skipped file · ${String(o.target_id)}` : 'Skipped file',
    });
  }
  if (rows.length > 0) return rows;
  return [];
}

function scanRowsFromSummary(objs: Record<string, unknown>[]): AuditRow[] {
  const scan = [...objs].reverse().find(isScanSummaryRecord);
  if (!scan) return [];
  const targets = Array.isArray(scan.targets) ? (scan.targets as Record<string, unknown>[]) : [];
  if (targets.length === 0) return [{
    type: 'Category',
    category: 'System Scan',
    location: `${scan.categories_count ?? scan.categoriesCount ?? 0} categories`,
    size: rowSizeText(scan.total_reclaimable_bytes ?? scan.eligible_bytes ?? scan.totalBytes ?? 0),
    outcome: 'Safe',
    outcomeTone: 'eligible',
    kind: 'Category',
    sublabel: 'Category',
  }];
  return targets.map((t) => {
    const files = Number(t.eligible_count ?? t.file_count ?? 0);
    const name = String(t.target_name ?? t.target_id ?? 'Target');
    return {
      type: 'Category',
      category: name,
      location: `${files} ${files === 1 ? 'file' : 'files'} found`,
      size: rowSizeText(t.eligible_bytes ?? t.total_bytes ?? 0),
      outcome: 'Safe',
      outcomeTone: 'eligible',
      kind: 'Category',
      sublabel: t.tier ? `Category · ${String(t.tier)}` : 'Category',
    } as AuditRow;
  });
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
function fallbackScanRow(outcome: Record<string, unknown> | null): AuditRow {
  const scan = outcome && isScanSummaryRecord(outcome) ? outcome : null;
  const cats = scan?.categories_count ?? scan?.categoriesCount ?? 0;
  const size = scan ? (scan.total_reclaimable_bytes ?? scan.eligible_bytes ?? scan.totalBytes ?? 0) : 0;
  return { type: 'Category', category: 'System Scan', location: `${cats} categories`, size: rowSizeText(size), outcome: 'Safe', outcomeTone: 'eligible', kind: 'Category', sublabel: 'Category' };
}
function fallbackRows(flags: ReturnType<typeof detectCleanScan>): AuditRow[] {
  if (flags.isClean) return [fallbackCleanRow(flags.outcome, flags.intent)];
  if (flags.isScan) return [fallbackScanRow(flags.outcome)];
  return [{ type: 'Category', category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral', kind: 'Category', sublabel: 'Category' }];
}

export function parseAuditRows(content: string): AuditRow[] {
  const objs = parseLogObjects(content);
  if (objs.length === 0) return [];
  const flags = detectCleanScan(objs);
  if (flags.isScan) {
    const scanRows = scanRowsFromSummary(objs);
    if (scanRows.length > 0) return scanRows;
  }
  if (flags.isClean) {
    const cleanRows = cleanRowsFromJournal(objs);
    if (cleanRows.length > 0) return cleanRows;
  }
  const itemRows = objs.filter(isItemRow);
  if (itemRows.length > 0) return itemRows.map((o) => normalizeItemRow(o, flags));
  return fallbackRows(flags);
}
