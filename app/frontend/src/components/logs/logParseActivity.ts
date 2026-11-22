import type { ActivitySummary, AuditRow, Flags } from './logParseTypes';
import { aggregateDeletedOutcomes, bytesToString, cleanOutcomeRecords, countFrom, detectCleanScan, formatMaybeBytes, isScanSummaryRecord, parseLogObjects } from './logParseUtils';
import { parseAuditRows } from './logParseRows';
import { pluralize } from '../../utils/format';

function emptyActivity(): ActivitySummary {
  return { primaryLabel: 'Space Reclaimed', primaryValue: '—', eligible: '—', skipped: '—', outcome: '—', mode: null };
}
function rowParentLabel(r: AuditRow): string { if (r.sublabel) return r.sublabel; if (r.category) return r.category; return ''; }
function countSkippedCategories(skippedFileRows: AuditRow[]): number {
  const parents = skippedFileRows.map((r) => rowParentLabel(r).split('·')[0].trim()).filter(Boolean);
  const unique = new Set(parents).size;
  if (unique > 0) return unique;
  return skippedFileRows.length;
}

function cleanActivity(objs: Record<string, unknown>[]): ActivitySummary {
  const outcomes = cleanOutcomeRecords(objs);
  const aggregates = aggregateDeletedOutcomes(outcomes);
  let freed = 0n;
  let deletedCount = 0;
  let skippedCount = 0;
  const cleanedCats = new Set<string>();
  const skippedCats = new Set<string>();
  for (const o of outcomes) {
    const status = String(o.status ?? '').toLowerCase();
    const target = String(o.target_id ?? '');
    if (status === 'deleted') {
      deletedCount += 1;
      if (target) cleanedCats.add(target);
      // Issue #135: skip the repeating root outcome when summing bytes.
      if (!aggregates.has(o)) {
        const b = bytesToString(o.bytes ?? o.size ?? 0) ?? '0';
        try { freed += BigInt(b); } catch { /* ignore */ }
      }
    } else if (status === 'skipped' || status === 'aborted') {
      skippedCount += 1;
      if (target) skippedCats.add(target);
    }
  }
  const rows = parseAuditRowsFromObjs(objs);
  const skippedRows = rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
  const skippedCatsCount = skippedCats.size || countSkippedCategories(skippedRows);
  return {
    primaryLabel: 'Space Reclaimed',
    primaryValue: formatMaybeBytes(freed.toString()),
    eligible: `${pluralize({ count: deletedCount, singular: 'file', plural: 'files' })} · ${pluralize({ count: cleanedCats.size, singular: 'category', plural: 'categories' })}`,
    skipped: `${pluralize({ count: skippedCount, singular: 'file', plural: 'files' })} · ${pluralize({ count: skippedCatsCount, singular: 'category', plural: 'categories' })}`,
    outcome: 'Completed',
    mode: 'Permanent Delete',
  };
}

function parseAuditRowsFromObjs(objs: Record<string, unknown>[]): AuditRow[] {
  const content = objs.map((o) => JSON.stringify(o)).join('\n');
  return parseAuditRows(content);
}

function scanActivity(objs: Record<string, unknown>[]): ActivitySummary {
  const rev = [...objs].reverse();
  const scan = rev.find(isScanSummaryRecord)
    ?? rev.find((o) => String(o.record ?? '') === 'outcome')
    ?? {};
  const reclaim = scan.total_reclaimable_bytes ?? scan.eligible_bytes ?? scan.totalBytes ?? '0';
  const files = countFrom(scan.total_eligible_files ?? scan.eligible_count);
  const skipped = countFrom(scan.total_skipped_files);
  const cats = countFrom(scan.categories_count ?? scan.categoriesCount ?? (Array.isArray(scan.targets) ? scan.targets.length : 0));
  return {
    primaryLabel: 'Space Found',
    primaryValue: formatMaybeBytes(reclaim),
    eligible: `${pluralize({ count: files, singular: 'file', plural: 'files' })} · ${pluralize({ count: cats, singular: 'category', plural: 'categories' })}`,
    skipped: `${pluralize({ count: skipped, singular: 'file', plural: 'files' })} · ${pluralize({ count: 0, singular: 'category', plural: 'categories' })}`,
    outcome: 'Completed',
    mode: null,
  };
}

function legacyActivity(objs: Record<string, unknown>[], flags: Flags): ActivitySummary {
  const outcome = [...objs].reverse().find((o) => String(o.record ?? '') === 'outcome') ?? undefined;
  if (flags.isScan) {
    return {
      primaryLabel: 'Space Found',
      primaryValue: formatMaybeBytes(outcome?.totalBytes),
      eligible: '—',
      skipped: '—',
      outcome: 'Completed',
      mode: null,
    };
  }
  return {
    primaryLabel: 'Space Reclaimed',
    primaryValue: formatMaybeBytes(outcome?.bytesFreed),
    eligible: '—',
    skipped: '—',
    outcome: 'Completed',
    mode: flags.isClean ? 'Permanent Delete' : null,
  };
}

function buildActivity(content: string): ActivitySummary {
  const objs = parseLogObjects(content);
  const flags = detectCleanScan(objs);
  if (flags.isScan || objs.some(isScanSummaryRecord)) return scanActivity(objs);
  if (flags.isClean) {
    if (cleanOutcomeRecords(objs).length > 0) return cleanActivity(objs);
    return legacyActivity(objs, flags);
  }
  return emptyActivity();
}
export function deriveActivitySummary(content: string | null): ActivitySummary {
  if (!content) return emptyActivity();
  try { return buildActivity(content); } catch { return { ...emptyActivity(), outcome: 'Recorded' }; }
}
