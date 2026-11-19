import type { AuditRow } from './logParseTypes';
import { cleanOutcomeRecords, countFrom, detectCleanScan, isScanSummaryRecord, parseLogObjects, rowSizeText } from './logParseUtils';
import { pluralize } from '../../utils/format';
import {
  addCleanedAggregate,
  cleanedTargetRow,
  isItemRow,
  normalizeItemRow,
  scanSummaryFallbackRow,
  scanTargetRow,
  skippedCleanRow,
} from './logParseRowBuilders';

function scanRowsFromSummary(objs: Record<string, unknown>[]): AuditRow[] {
  const scan = [...objs].reverse().find(isScanSummaryRecord);
  if (!scan) return [];
  const targets = Array.isArray(scan.targets) ? (scan.targets as Record<string, unknown>[]) : [];
  if (targets.length === 0) return [scanSummaryFallbackRow(scan)];
  return targets.map(scanTargetRow);
}

function collectCleanedAggregates(outcomes: Record<string, unknown>[]): Map<string, { bytes: bigint; count: number }> {
  const map = new Map<string, { bytes: bigint; count: number }>();
  for (const o of outcomes) {
    if (String(o.status ?? '').toLowerCase() !== 'deleted') continue;
    addCleanedAggregate(map, o);
  }
  return map;
}

function appendSkippedCleanRows(rows: AuditRow[], outcomes: Record<string, unknown>[]) {
  for (const o of outcomes) {
    const row = skippedCleanRow(o);
    if (row) rows.push(row);
  }
}

function cleanRowsFromJournal(objs: Record<string, unknown>[]): AuditRow[] {
  const outcomes = cleanOutcomeRecords(objs);
  if (outcomes.length === 0) return [];
  const rows = [...collectCleanedAggregates(outcomes).entries()].map(([id, agg]) => cleanedTargetRow(id, agg));
  appendSkippedCleanRows(rows, outcomes);
  return rows;
}

function fallbackCleanLocation(outcome: Record<string, unknown> | null): string {
  const deleted = outcome?.deletedCount;
  if (deleted == null) return 'Completed cleanup';
  return `${pluralize({ count: countFrom(deleted), singular: 'item', plural: 'items' })} removed`;
}

function fallbackCleanRow(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): AuditRow {
  const category = outcome?.category != null ? String(outcome.category) : intent?.category != null ? String(intent.category) : 'Cleanup';
  const size = outcome?.bytesFreed == null ? '\u2014' : rowSizeText(outcome.bytesFreed);
  return { type: 'Category', category, location: fallbackCleanLocation(outcome), size, outcome: 'Cleaned', outcomeTone: 'cleaned', kind: 'Category', sublabel: 'Category' };
}

function fallbackScanRow(outcome: Record<string, unknown> | null): AuditRow {
  const scan = outcome && isScanSummaryRecord(outcome) ? outcome : null;
  const cats = countFrom(scan?.categories_count ?? scan?.categoriesCount);
  const size = scan ? scan.total_reclaimable_bytes ?? scan.eligible_bytes ?? scan.totalBytes ?? 0 : 0;
  return { type: 'Category', category: 'System Scan', location: pluralize({ count: cats, singular: 'category', plural: 'categories' }), size: rowSizeText(size), outcome: 'Safe', outcomeTone: 'eligible', kind: 'Category', sublabel: 'Category' };
}

function fallbackRows(flags: ReturnType<typeof detectCleanScan>): AuditRow[] {
  if (flags.isClean) return [fallbackCleanRow(flags.outcome, flags.intent)];
  if (flags.isScan) return [fallbackScanRow(flags.outcome)];
  return [{ type: 'Category', category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral', kind: 'Category', sublabel: 'Category' }];
}

function rowsForScan(objs: Record<string, unknown>[], flags: ReturnType<typeof detectCleanScan>): AuditRow[] | null {
  if (!flags.isScan) return null;
  const rows = scanRowsFromSummary(objs);
  if (rows.length === 0) return null;
  return rows;
}

function rowsForClean(objs: Record<string, unknown>[], flags: ReturnType<typeof detectCleanScan>): AuditRow[] | null {
  if (!flags.isClean) return null;
  const rows = cleanRowsFromJournal(objs);
  if (rows.length === 0) return null;
  return rows;
}

export function parseAuditRows(content: string): AuditRow[] {
  const objs = parseLogObjects(content);
  if (objs.length === 0) return [];
  const flags = detectCleanScan(objs);
  const scanRows = rowsForScan(objs, flags);
  if (scanRows) return scanRows;
  const cleanRows = rowsForClean(objs, flags);
  if (cleanRows) return cleanRows;
  const itemRows = objs.filter(isItemRow);
  if (itemRows.length > 0) return itemRows.map((o) => normalizeItemRow(o, flags));
  return fallbackRows(flags);
}
