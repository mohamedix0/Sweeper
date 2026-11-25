import { formatBytes, formatUnixSeconds, relativeAge } from '../../utils/format';
import type { BaseSummary, Flags, LogMeta } from './logParseTypes';

function isRecentEpoch(args: { value: number }): boolean { return args.value > 1e9; }
function numberIfRecent(args: { n: number }): number | null { if (isRecentEpoch({ value: args.n })) return args.n; return null; }
function finiteNumber(args: { raw: unknown }): number | null { if (typeof args.raw !== 'number') return null; if (Number.isFinite(args.raw)) return args.raw; return null; }
function parseStringNumber(args: { raw: string }): number | null { const n = Number(args.raw); if (Number.isFinite(n)) return n; return null; }
export function toFiniteTime(raw: unknown): number | null {
  const num = finiteNumber({ raw });
  if (num != null) return numberIfRecent({ n: num });
  if (typeof raw !== 'string') return null;
  const parsed = parseStringNumber({ raw });
  if (parsed == null) return null;
  return numberIfRecent({ n: parsed });
}
function epochMultiplier(args: { value: number }): number { if (args.value < 1e12) return 1000; return 1; }
function epochDate(args: { value: number }): Date { return new Date(args.value * epochMultiplier({ value: args.value })); }
export function formatEpochTime(value: number): string | null { const d = epochDate({ value }); if (Number.isNaN(d.getTime())) return null; return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }); }
export function rawClockTime(raw: unknown): string | null { if (typeof raw !== 'string') return null; const trimmed = raw.trim(); if (/^\d{1,2}:\d{2}/.test(trimmed)) return trimmed; return null; }
export function metaClockTime(meta: LogMeta | null): string | null { if (!meta?.modifiedUnixSecs) return null; return formatEpochTime(Number(meta.modifiedUnixSecs)); }
export function directClockText(raw: unknown): string | null { const direct = toFiniteTime(raw); if (direct == null) return null; return formatEpochTime(direct); }
export function formatClockTime(meta: LogMeta | null, raw?: unknown): string {
  const directText = directClockText(raw); if (directText) return directText;
  const rawText = rawClockTime(raw); if (rawText) return rawText;
  const metaText = metaClockTime(meta); if (metaText) return metaText;
  return '';
}
function formatLogDate(meta: LogMeta): string {
  const n = Number(meta.modifiedUnixSecs);
  if (!Number.isFinite(n)) return formatUnixSeconds({ secs: meta.modifiedUnixSecs });
  const d = epochDate({ value: n });
  if (Number.isNaN(d.getTime())) return formatUnixSeconds({ secs: meta.modifiedUnixSecs });
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' });
}
export function formatLogWhen(meta: LogMeta): string {
  const dash = '\u2014'; if (!meta.modifiedUnixSecs) return dash;
  const rel = relativeAge({ secs: meta.modifiedUnixSecs }); if (rel !== dash) return rel;
  try { return formatLogDate(meta); } catch { return formatUnixSeconds({ secs: meta.modifiedUnixSecs }); }
}
function pushLine(args: { objs: Record<string, unknown>[]; line: string }) {
  const t = args.line.trim();
  if (!t) return;
  try { args.objs.push(JSON.parse(t) as Record<string, unknown>); } catch { /* ignore */ }
}
export function parseLogObjects(content: string): Record<string, unknown>[] {
  const objs: Record<string, unknown>[] = [];
  for (const line of content.split(/\r?\n/)) pushLine({ objs, line });
  return objs;
}

export function isScanSummaryRecord(o: Record<string, unknown>): boolean {
  if (String(o.kind) === 'scan') return true;
  if (String(o.event) === 'scan') return true;
  if (o.action === 'scan') return true;
  return false;
}

function isPerItemOutcomeStatus(status: string): boolean {
  if (status === 'deleted' || status === 'skipped') return true;
  return status === 'aborted';
}

function hasOutcomeItemRef(o: Record<string, unknown>): boolean {
  if (o.path != null) return true;
  return o.target_id != null;
}

export function isJournalPerItemOutcome(o: Record<string, unknown>): boolean {
  const rec = String(o.record ?? '');
  if (rec !== 'outcome') return false;
  if (isScanSummaryRecord(o)) return false;
  const status = String(o.status ?? '').toLowerCase();
  if (!isPerItemOutcomeStatus(status)) return false;
  return hasOutcomeItemRef(o);
}

export function cleanOutcomeRecords(objs: Record<string, unknown>[]): Record<string, unknown>[] {
  return objs.filter((o) => isJournalPerItemOutcome(o));
}

/** Normalize a log path for byte-index keys (both separators, case-insensitive). */
function normalizeLogPath(raw: string): string {
  return raw.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
}

/** Every strict ancestor of a normalized path (separator boundary, no self). */
function strictAncestorKeys(normalized: string): string[] {
  if (!normalized) return [];
  const parts = normalized.split('\\');
  const keys: string[] = [];
  for (let count = 1; count < parts.length; count += 1) {
    keys.push(parts.slice(0, count).join('\\'));
  }
  return keys;
}

function bytesToBig(raw: unknown): bigint {
  const text = bytesToString(raw);
  if (text == null) return 0n;
  try { return BigInt(text); } catch { return 0n; }
}

/**
 * Issue #135: a Tier 3 / WholeCache root writes one `deleted` outcome per child
 * plus a root outcome whose `bytes` repeats the sum of those children. Summing
 * every `deleted` outcome double-counts reclaimed bytes.
 *
 * Return each deleted outcome's own byte contribution: for a root, its bytes
 * minus the bytes of journaled descendants (mirroring the bridge's
 * `root_contribution`, `bytes.saturating_sub(descendants)`). When every child
 * was journaled the root contributes zero; when a child write failed, only the
 * missing bytes are recovered instead of discarding the whole root. Ordinary
 * outcomes contribute their own bytes unchanged. Paths are normalized once and
 * descendant bytes are indexed per target ancestor, so this is one lookup per
 * outcome with no O(n^2) scan.
 */
export function aggregateDeletedOutcomes(
  outcomes: Record<string, unknown>[],
): Map<Record<string, unknown>, bigint> {
  const deleted = outcomes.filter((o) => String(o.status ?? '').toLowerCase() === 'deleted');
  const byTarget = new Map<string, Map<string, bigint>>();
  const normalizedPaths = new Map<Record<string, unknown>, string>();

  for (const outcome of deleted) {
    const target = String(outcome.target_id ?? '');
    const normalized = normalizeLogPath(String(outcome.path ?? ''));
    normalizedPaths.set(outcome, normalized);
    let index = byTarget.get(target);
    if (!index) {
      index = new Map<string, bigint>();
      byTarget.set(target, index);
    }
    const bytes = bytesToBig(outcome.bytes ?? outcome.size ?? 0);
    for (const ancestor of strictAncestorKeys(normalized)) {
      index.set(ancestor, (index.get(ancestor) ?? 0n) + bytes);
    }
  }

  const contributions = new Map<Record<string, unknown>, bigint>();
  for (const outcome of deleted) {
    const bytes = bytesToBig(outcome.bytes ?? outcome.size ?? 0);
    const path = normalizedPaths.get(outcome) ?? '';
    const target = String(outcome.target_id ?? '');
    const descendantBytes = path ? byTarget.get(target)?.get(path) ?? 0n : 0n;
    // A root with no journaled descendants is an ordinary deletion. When
    // descendants exist, keep only the part of its bytes they do not cover.
    const contribution = descendantBytes === 0n
      ? bytes
      : bytes > descendantBytes ? bytes - descendantBytes : 0n;
    contributions.set(outcome, contribution);
  }
  return contributions;
}

export function outcomeAndIntent(objs: Record<string, unknown>[]) {
  const outcomes = objs.filter((o) => String(o.record ?? '') === 'outcome');
  const summary = [...outcomes].reverse().find((o) => isScanSummaryRecord(o) || o.bytesFreed != null || o.deletedCount != null || o.total_reclaimable_bytes != null || o.totalBytes != null) ?? null;
  const perItem = outcomes.find((o) => isJournalPerItemOutcome(o)) ?? null;
  const outcome = summary ?? perItem ?? outcomes[outcomes.length - 1] ?? null;
  const intent = objs.find((o) => String(o.record ?? '') === 'intent') ?? null;
  return { outcome, intent };
}
function isCleanEvent(args: { o: Record<string, unknown> }): boolean { if (args.o.event === 'clean') return true; return args.o.action === 'clean'; }
function hasCleanCounts(args: { o: Record<string, unknown> }): boolean { if (args.o.deletedCount != null) return true; return args.o.bytesFreed != null; }
function isCleanSummaryRecord(o: Record<string, unknown> | null): boolean { if (!o) return false; if (isScanSummaryRecord(o)) return false; if (isCleanEvent({ o })) return true; return hasCleanCounts({ o }); }
function isScanEvent(args: { o: Record<string, unknown> }): boolean { if (args.o.event === 'scan') return true; return args.o.action === 'scan'; }
function hasScanStats(args: { o: Record<string, unknown> }): boolean {
  if (args.o.totalBytes != null) return true;
  if (args.o.total_reclaimable_bytes != null) return true;
  if (args.o.categoriesCount != null) return true;
  if (args.o.categories_count != null) return true;
  return args.o.total_eligible_files != null;
}
function isScanRecord(o: Record<string, unknown> | null): boolean { if (!o) return false; if (isScanEvent({ o })) return true; return hasScanStats({ o }); }
function scanFromRecords(args: { outcome: Record<string, unknown> | null; intent: Record<string, unknown> | null }): boolean {
  if (isScanRecord(args.outcome)) return true;
  return isScanRecord(args.intent);
}
export function detectCleanScan(objs: Record<string, unknown>[]): { outcome: Record<string, unknown> | null; intent: Record<string, unknown> | null; isClean: boolean; isScan: boolean } {
  const { outcome, intent } = outcomeAndIntent(objs);
  const hasPerItemClean = cleanOutcomeRecords(objs).length > 0;
  let clean = hasPerItemClean || isCleanSummaryRecord(outcome) || isCleanSummaryRecord(intent);
  let scan = false;
  if (!clean) scan = scanFromRecords({ outcome, intent }) || objs.some(isScanSummaryRecord);
  if (objs.some(isScanSummaryRecord)) { scan = true; clean = false; }
  return { outcome, intent, isClean: clean, isScan: scan };
}
export function baseSummary(meta: LogMeta): BaseSummary { const when = formatLogWhen(meta); const clock = formatClockTime(meta); if (clock) return { when, whenAt: `${when} at ${clock}` }; return { when, whenAt: when }; }
export function rowSizeText(rawSize: unknown): string {
  const dash = '\u2014'; if (rawSize == null) return dash;
  if (/^\d+$/.test(String(rawSize))) return formatBytes({ bytes: String(rawSize) });
  const text = String(rawSize); if (text) return text; return dash;
}
export function bytesToString(raw: unknown): string | null {
  if (typeof raw === 'string' && /^\d+$/.test(raw)) return raw;
  if (typeof raw === 'number' && Number.isFinite(raw)) return String(Math.max(0, Math.trunc(raw)));
  if (typeof raw === 'string') {
    const n = Number(raw);
    if (Number.isFinite(n)) return String(Math.max(0, Math.trunc(n)));
  }
  return null;
}
/** Coerce a log count field to a non-negative integer for pluralization. */
export function countFrom(raw: unknown): number {
  const text = bytesToString(raw);
  if (text == null) return 0;
  const value = Number(text);
  return Number.isFinite(value) ? value : 0;
}
export function formatMaybeBytes(raw: unknown): string {
  const s = bytesToString(raw);
  if (s != null) return formatBytes({ bytes: s });
  return '\u2014';
}
export function outcomeBytes(outcome: Record<string, unknown> | undefined, key: 'bytesFreed' | 'totalBytes'): string {
  const val = outcome?.[key]; if (typeof val === 'string') return formatBytes({ bytes: val }); if (typeof val === 'number') return formatBytes({ bytes: String(val) }); return '\u2014';
}
export function modeFor(flags: Flags): string | null { if (flags.isClean) return 'Permanent Delete'; return null; }
