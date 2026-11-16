import type { AuditRow, Flags } from './logParseTypes';
import { bytesToString, isJournalPerItemOutcome, rowSizeText } from './logParseUtils';
import { normalizeOutcome } from './logParseOutcome';

function isScanSummaryEvent(ev: string, o: Record<string, unknown>): boolean {
  if (ev !== 'scan') return false;
  if (o.categoriesCount != null) return true;
  if (o.categories_count != null) return true;
  if (o.totalBytes != null) return true;
  if (o.total_reclaimable_bytes != null) return true;
  return Array.isArray(o.targets);
}

function rowRecordIgnored(rec: string, o: Record<string, unknown>): boolean {
  if (isJournalPerItemOutcome(o)) return false;
  if (rec === 'outcome') return true;
  return rec === 'intent';
}

function rowHasKnownField(o: Record<string, unknown>): boolean {
  if (o.category != null) return true;
  if (o.path != null) return true;
  if (o.target != null) return true;
  if (o.size != null) return true;
  if (o.bytes != null) return true;
  if (o.target_id != null) return true;
  return Boolean(o.status);
}

export function isItemRow(o: Record<string, unknown>): boolean {
  const rec = String(o.record ?? '');
  if (rowRecordIgnored(rec, o)) return false;
  const ev = String(o.event ?? o.action ?? '').toLowerCase();
  if (isScanSummaryEvent(ev, o)) return false;
  return rowHasKnownField(o);
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
  const category = parts[parts.length - 1] || rawCategory;
  return { category, sublabel: skippedParentLabel(rawCategory, location) };
}

function outcomeDefault(flags: Flags): 'cleaned' | 'eligible' | 'neutral' {
  if (flags.isClean) return 'cleaned';
  if (flags.isScan) return 'eligible';
  return 'neutral';
}

function outcomeRaw(o: Record<string, unknown>, flags: Flags): unknown {
  if (o.status != null) return o.status;
  if (flags.isClean) return 'Cleaned';
  if (flags.isScan) return 'Eligible';
  return 'Recorded';
}

function itemOutcome(o: Record<string, unknown>, flags: Flags) {
  return normalizeOutcome({ raw: outcomeRaw(o, flags), fallback: outcomeDefault(flags) });
}

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
  if (tone === 'skipped') return targetId ? `Skipped file · ${String(targetId)}` : 'Skipped file';
  if (targetId) return String(targetId);
  return undefined;
}

function rowType(tone: AuditRow['outcomeTone']): string {
  if (tone === 'skipped') return 'Skipped file';
  return 'Category';
}

function skippedForRow(kind: 'Category' | 'File', rawCategory: string, location: string, tone: AuditRow['outcomeTone']) {
  if (kind !== 'File') return null;
  return resolveSkippedFileRow(rawCategory, location, tone);
}

export function normalizeItemRow(o: Record<string, unknown>, flags: Flags): AuditRow {
  const rawCategory = itemCategory(o, flags);
  const location = String(o.path ?? o.details ?? o.location ?? '—');
  const size = rowSizeText(o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes ?? o.eligible_bytes);
  const outcome = itemOutcome(o, flags);
  const kind = itemKind(o);
  const skipped = skippedForRow(kind, rawCategory, location, outcome.tone);
  const category = skipped ? skipped.category : rawCategory;
  return {
    type: rowType(outcome.tone),
    category,
    location,
    size,
    outcome: outcome.label,
    outcomeTone: outcome.tone,
    kind,
    sublabel: itemSublabel(kind, skipped, outcome.tone, o.target_id),
  };
}

export function cleanedTargetRow(targetId: string, agg: { bytes: bigint; count: number }): AuditRow {
  const countLabel = agg.count === 1 ? 'item' : 'items';
  return {
    type: 'Category',
    category: targetId,
    location: `${agg.count} ${countLabel} cleaned`,
    size: rowSizeText(agg.bytes.toString()),
    outcome: 'Cleaned',
    outcomeTone: 'cleaned',
    kind: 'Category',
    sublabel: 'Category',
  };
}

function skippedItemLabel(o: Record<string, unknown>): string {
  const path = String(o.path ?? o.target_id ?? 'Item');
  return path.split(/[\\/]/).filter(Boolean).pop() ?? 'Item';
}

function skippedOutcomeLabel(reason: string): string {
  if (reason) return `Skipped · ${reason}`;
  return 'Skipped';
}

export function skippedCleanRow(o: Record<string, unknown>): AuditRow | null {
  const status = String(o.status ?? '').toLowerCase();
  if (status !== 'skipped' && status !== 'aborted') return null;
  const reason = String(o.reason ?? '');
  return {
    type: 'Skipped file',
    category: skippedItemLabel(o),
    location: String(o.path ?? '—'),
    size: rowSizeText(o.bytes ?? o.size ?? 0),
    outcome: skippedOutcomeLabel(reason),
    outcomeTone: 'skipped',
    kind: 'File',
    sublabel: o.target_id ? `Skipped file · ${String(o.target_id)}` : 'Skipped file',
  };
}

export function addCleanedAggregate(map: Map<string, { bytes: bigint; count: number }>, o: Record<string, unknown>) {
  const targetId = String(o.target_id ?? 'Cleanup');
  const cur = map.get(targetId) ?? { bytes: 0n, count: 0 };
  const b = bytesToString(o.bytes ?? 0) ?? '0';
  try {
    cur.bytes += BigInt(b);
  } catch {
    /* ignore */
  }
  cur.count += 1;
  map.set(targetId, cur);
}

export function scanTargetRow(t: Record<string, unknown>): AuditRow {
  const files = Number(t.eligible_count ?? t.file_count ?? 0);
  const filesLabel = files === 1 ? 'file' : 'files';
  const name = String(t.target_name ?? t.target_id ?? 'Target');
  return {
    type: 'Category',
    category: name,
    location: `${files} ${filesLabel} found`,
    size: rowSizeText(t.eligible_bytes ?? t.total_bytes ?? 0),
    outcome: 'Safe',
    outcomeTone: 'eligible',
    kind: 'Category',
    sublabel: t.tier ? `Category · ${String(t.tier)}` : 'Category',
  };
}

export function scanSummaryFallbackRow(scan: Record<string, unknown>): AuditRow {
  return {
    type: 'Category',
    category: 'System Scan',
    location: `${scan.categories_count ?? scan.categoriesCount ?? 0} categories`,
    size: rowSizeText(scan.total_reclaimable_bytes ?? scan.eligible_bytes ?? scan.totalBytes ?? 0),
    outcome: 'Safe',
    outcomeTone: 'eligible',
    kind: 'Category',
    sublabel: 'Category',
  };
}
