import { formatBytes, formatUnixSeconds, relativeAge } from '../../utils/format';

export type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
export type AuditRow = { type: string; category: string; location: string; size: string; outcome: string; outcomeTone: 'eligible' | 'cleaned' | 'skipped' | 'neutral'; kind: 'Category' | 'File'; sublabel?: string };
export type LogFilter = 'all' | 'scan' | 'clean';
export type OutcomeFilter = 'all' | 'eligible' | 'skipped';

function toFiniteTime(raw: unknown): number | null {
  if (typeof raw === 'number' && Number.isFinite(raw) && raw > 1e9) return raw;
  if (typeof raw !== 'string') return null;
  const n = Number(raw);
  return Number.isFinite(n) && n > 1e9 ? n : null;
}

function epochDate(value: number): Date {
  return new Date(value * (value < 1e12 ? 1000 : 1));
}

function formatEpochTime(value: number): string | null {
  const d = epochDate(value);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

function rawClockTime(raw: unknown): string | null {
  if (typeof raw !== 'string') return null;
  const trimmed = raw.trim();
  return /^\d{1,2}:\d{2}/.test(trimmed) ? trimmed : null;
}

function metaClockTime(meta: LogMeta | null): string | null {
  if (!meta?.modifiedUnixSecs) return null;
  return formatEpochTime(Number(meta.modifiedUnixSecs));
}

export function formatClockTime(meta: LogMeta | null, raw?: unknown): string {
  const direct = toFiniteTime(raw);
  const directText = direct == null ? null : formatEpochTime(direct);
  if (directText) return directText;
  return rawClockTime(raw) ?? metaClockTime(meta) ?? '';
}

function formatLogDate(meta: LogMeta): string {
  const n = Number(meta.modifiedUnixSecs);
  if (!Number.isFinite(n)) return formatUnixSeconds(meta.modifiedUnixSecs);
  const d = epochDate(n);
  if (Number.isNaN(d.getTime())) return formatUnixSeconds(meta.modifiedUnixSecs);
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' });
}

export function formatLogWhen(meta: LogMeta): string {
  if (!meta.modifiedUnixSecs) return '—';
  const rel = relativeAge(meta.modifiedUnixSecs);
  if (rel !== '—') return rel;
  try { return formatLogDate(meta); } catch { return formatUnixSeconds(meta.modifiedUnixSecs); }
}

function skippedOutcomeLabel(s: string): { label: string; tone: AuditRow['outcomeTone'] } {
  if (s.includes('unknown') || s.includes('unverified')) return { label: 'Skipped · Unverified', tone: 'skipped' };
  if (s.includes('< 24') || s.includes('24h')) return { label: 'Skipped · < 24h', tone: 'skipped' };
  return { label: 'Skipped', tone: 'skipped' };
}

function fallbackOutcome(fallback: 'eligible' | 'cleaned' | 'skipped' | 'neutral', raw: unknown) {
  if (fallback === 'cleaned') return { label: 'Cleaned', tone: 'cleaned' as const };
  if (fallback === 'eligible') return { label: 'Safe', tone: 'eligible' as const };
  if (fallback === 'skipped') return { label: 'Skipped', tone: 'skipped' as const };
  return { label: String(raw || 'Recorded'), tone: 'neutral' as const };
}

export function normalizeOutcome(raw: unknown, fallback: 'eligible' | 'cleaned' | 'skipped' | 'neutral'): { label: string; tone: AuditRow['outcomeTone'] } {
  const s = String(raw ?? '').toLowerCase();
  if (s.includes('clean')) return { label: 'Cleaned', tone: 'cleaned' };
  if (s.includes('eligible') || s.includes('safe')) return { label: 'Safe', tone: 'eligible' };
  if (s.includes('skip')) return skippedOutcomeLabel(s);
  return fallbackOutcome(fallback, raw);
}

function parseLogObjects(content: string): Record<string, unknown>[] {
  const objs: Record<string, unknown>[] = [];
  for (const line of content.split(/\r?\n/)) {
    const t = line.trim();
    if (!t) continue;
    try { objs.push(JSON.parse(t) as Record<string, unknown>); } catch { /* ignore */ }
  }
  return objs;
}

function outcomeAndIntent(objs: Record<string, unknown>[]) {
  const outcome = [...objs].reverse().find((o) => o.record === 'outcome') ?? null;
  const intent = objs.find((o) => o.record === 'intent') ?? null;
  return { outcome, intent };
}

function isCleanRecord(o: Record<string, unknown> | null): boolean {
  if (!o) return false;
  if (o.event === 'clean' || o.action === 'clean') return true;
  return o.deletedCount != null || o.bytesFreed != null;
}

function isScanRecord(o: Record<string, unknown> | null): boolean {
  if (!o) return false;
  if (o.event === 'scan' || o.action === 'scan') return true;
  return o.totalBytes != null || o.categoriesCount != null;
}

function detectCleanScan(objs: Record<string, unknown>[]) {
  const { outcome, intent } = outcomeAndIntent(objs);
  const isClean = isCleanRecord(outcome) || isCleanRecord(intent);
  const isScan = !isClean && (isScanRecord(outcome) || isScanRecord(intent));
  return { outcome, intent, isClean, isScan };
}

function isScanSummaryEvent(ev: string, o: Record<string, unknown>): boolean {
  return ev === 'scan' && (o.categoriesCount != null || o.totalBytes != null);
}

function isItemRow(o: Record<string, unknown>): boolean {
  const rec = String(o.record ?? '');
  if (rec === 'outcome' || rec === 'intent') return false;
  const ev = String(o.event ?? o.action ?? '').toLowerCase();
  if (isScanSummaryEvent(ev, o)) return false;
  return !!(o.category || o.path || o.target || o.size || o.bytes || o.status);
}

function rowSizeText(rawSize: unknown): string {
  if (rawSize == null) return '—';
  if (/^\d+$/.test(String(rawSize))) return formatBytes(String(rawSize));
  return String(rawSize) || '—';
}

type Flags = { isClean: boolean; isScan: boolean };

function skippedParentLabel(rawCategory: string, location: string): string {
  if (rawCategory !== 'Custom Path (Unknown)') return rawCategory;
  const parts = location.split(/[\\/]/).filter(Boolean);
  return `Custom Path · ${(parts[parts.length - 2] || 'CustomCleanup')}`;
}

function resolveSkippedFileRow(rawCategory: string, location: string, tone: AuditRow['outcomeTone']) {
  if (tone !== 'skipped' || !location || location === '—') return null;
  const parts = location.split(/[\\/]/).filter(Boolean);
  const category = parts[parts.length - 1] || rawCategory;
  return { category, sublabel: skippedParentLabel(rawCategory, location) };
}

function itemOutcome(o: Record<string, unknown>, flags: Flags) {
  const fallback = flags.isClean ? 'cleaned' : flags.isScan ? 'eligible' : 'neutral';
  return normalizeOutcome(o.status ?? (flags.isClean ? 'Cleaned' : flags.isScan ? 'Eligible' : 'Recorded'), fallback);
}

function itemCategory(o: Record<string, unknown>, flags: Flags): string {
  return String(o.category ?? o.target ?? o.action ?? (flags.isScan ? 'Scan' : 'Cleanup'));
}

function itemKind(o: Record<string, unknown>): 'Category' | 'File' {
  return o.kind === 'Category' || o.kind === 'File' ? (o.kind as 'Category' | 'File') : 'File';
}

function itemSublabel(kind: 'Category' | 'File', skipped: { sublabel: string } | null, tone: AuditRow['outcomeTone']) {
  if (kind === 'Category') return undefined;
  return skipped?.sublabel ?? (tone === 'skipped' ? 'Skipped file' : undefined);
}

function normalizeItemRow(o: Record<string, unknown>, flags: Flags): AuditRow {
  const rawCategory = itemCategory(o, flags);
  const location = String(o.path ?? o.details ?? o.location ?? '—');
  const size = rowSizeText(o.size ?? o.bytes ?? o.bytesFreed ?? o.totalBytes);
  const outcome = itemOutcome(o, flags);
  const kind = itemKind(o);
  const skipped = kind === 'File' ? resolveSkippedFileRow(rawCategory, location, outcome.tone) : null;
  const category = skipped?.category ?? rawCategory;
  const sublabel = itemSublabel(kind, skipped, outcome.tone);
  const type = outcome.tone === 'skipped' ? 'Skipped file' : 'Category';
  return { type, category, location, size, outcome: outcome.label, outcomeTone: outcome.tone, kind, sublabel };
}

function fallbackCleanRow(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): AuditRow {
  const category = String(outcome?.category ?? intent?.category ?? 'Cleanup');
  const deleted = outcome?.deletedCount != null ? Number(outcome.deletedCount) : null;
  const freed = outcome?.bytesFreed != null ? formatBytes(String(outcome.bytesFreed)) : '—';
  const location = deleted != null ? `${deleted} ${deleted === 1 ? 'item' : 'items'} removed` : 'Completed cleanup';
  return { type: 'Category', category, location, size: freed, outcome: 'Cleaned', outcomeTone: 'cleaned', kind: 'Category', sublabel: 'Category' };
}

function fallbackScanRow(outcome: Record<string, unknown> | null, intent: Record<string, unknown> | null): AuditRow {
  const cats = outcome?.categoriesCount ?? intent?.categoriesCount;
  const total = outcome?.totalBytes ?? intent?.totalBytes;
  const items = outcome?.items ?? intent?.items;
  const catText = `${cats ?? '—'} ${Number(cats) === 1 ? 'category' : 'categories'}`;
  const location = items != null ? `${catText} · ${items} items` : catText;
  return { type: 'Category', category: 'System Scan', location, size: total != null ? formatBytes(String(total)) : '—', outcome: 'Safe', outcomeTone: 'eligible', kind: 'Category', sublabel: 'Category' };
}

function fallbackRows(flags: ReturnType<typeof detectCleanScan>): AuditRow[] {
  if (flags.isClean) return [fallbackCleanRow(flags.outcome, flags.intent)];
  if (flags.isScan) return [fallbackScanRow(flags.outcome, flags.intent)];
  return [{ type: 'Category', category: 'Activity', location: '—', size: '—', outcome: 'Recorded', outcomeTone: 'neutral', kind: 'Category', sublabel: 'Category' }];
}

export function parseAuditRows(content: string, _meta: LogMeta | null): AuditRow[] {
  const objs = parseLogObjects(content);
  if (objs.length === 0) return [];
  const flags = detectCleanScan(objs);
  const itemRows = objs.filter(isItemRow);
  if (itemRows.length > 0) return itemRows.map((o) => normalizeItemRow(o, flags));
  return fallbackRows(flags);
}

type LogSummary = { title: string; subtitle: string; when: string; whenAt: string; kind: LogFilter };
type BaseSummary = Pick<LogSummary, 'when' | 'whenAt'>;

function baseSummary(meta: LogMeta): BaseSummary {
  const when = formatLogWhen(meta);
  const clock = formatClockTime(meta);
  return { when, whenAt: clock ? `${when} at ${clock}` : when };
}

function cleanTitle(outcome: Record<string, unknown>): string {
  const deleted = outcome.deletedCount ?? outcome.deleted;
  const deletedStr = deleted != null ? `${String(deleted)} item${String(deleted) === '1' ? '' : 's'}` : null;
  const hasBytes = typeof outcome.bytesFreed === 'string' || typeof outcome.bytesFreed === 'number';
  const freed = hasBytes ? formatBytes(String(outcome.bytesFreed)) : null;
  if (freed && deletedStr) return `Cleaned ${freed} · ${deletedStr}`;
  if (deletedStr) return `Cleaned ${deletedStr}`;
  if (freed) return `Cleaned ${freed}`;
  return 'Clean Operation';
}

function summarizeCleanOutcome(outcome: Record<string, unknown>, base: BaseSummary): LogSummary {
  return { title: cleanTitle(outcome), subtitle: 'Completed', ...base, kind: 'clean' };
}

function scanTitle(outcome: Record<string, unknown>): string {
  const raw = outcome.categoriesCount;
  const count = raw == null ? null : Number(raw);
  const detail = count && count > 0 ? ` · ${count} ${count === 1 ? 'category' : 'categories'}` : '';
  return `System Scan${detail}`;
}

function summarizeScanOutcome(outcome: Record<string, unknown>, base: BaseSummary): LogSummary {
  return { title: scanTitle(outcome), subtitle: 'Completed', ...base, kind: 'scan' };
}

function isCleanOutcome(outcome: Record<string, unknown>): boolean {
  if (String(outcome.event) === 'scan' || outcome.action === 'scan') return false;
  if (outcome.deletedCount != null || outcome.bytesFreed != null) return true;
  return String(outcome.record) === 'outcome' && (outcome.event === 'clean' || outcome.action === 'clean');
}

function isScanOutcome(outcome: Record<string, unknown>): boolean {
  if (String(outcome.event) === 'scan' || outcome.action === 'scan') return true;
  if (String(outcome.record) !== 'outcome') return false;
  return (outcome.categoriesCount != null || outcome.totalBytes != null) && outcome.deletedCount == null;
}

function findSummaryOutcome(lines: Record<string, unknown>[]) {
  return [...lines].reverse().find((o) => o.record === 'outcome' || o.event === 'clean' || o.event === 'scan' || o.action === 'scan' || o.action === 'clean');
}

function summaryFromOutcome(outcome: Record<string, unknown> | undefined, base: BaseSummary): LogSummary | null {
  if (!outcome) return null;
  if (isCleanOutcome(outcome)) return summarizeCleanOutcome(outcome, base);
  if (isScanOutcome(outcome)) return summarizeScanOutcome(outcome, base);
  return null;
}

function summarizeFromContent(content: string | null, base: BaseSummary): LogSummary | null {
  if (!content) return null;
  try {
    const lines = content.split(/\r?\n/).filter(Boolean).map((l) => JSON.parse(l) as Record<string, unknown>);
    return summaryFromOutcome(findSummaryOutcome(lines), base);
  } catch { return null; }
}

function fallbackSummary(meta: LogMeta, base: BaseSummary): LogSummary {
  const lower = meta.name.toLowerCase();
  if (lower.includes('clean')) return { title: 'Clean Operation', subtitle: 'Completed', ...base, kind: 'clean' };
  if (lower.includes('scan')) return { title: 'System Scan', subtitle: 'Completed', ...base, kind: 'scan' };
  return { title: meta.name.replace(/\.jsonl$/i, ''), subtitle: 'Completed', ...base, kind: 'all' };
}

export function summarizeLog(meta: LogMeta, content: string | null): LogSummary {
  const base = baseSummary(meta);
  return summarizeFromContent(content, base) ?? fallbackSummary(meta, base);
}

type ActivitySummary = { primaryLabel: string; primaryValue: string; eligible: string; skipped: string; outcome: string; mode: string | null };

function outcomeBytes(outcome: Record<string, unknown> | undefined, key: 'bytesFreed' | 'totalBytes'): string {
  const val = outcome?.[key];
  const has = typeof val === 'string' || typeof val === 'number';
  return has ? formatBytes(String(val)) : '—';
}

function primaryMetric(flags: Flags, outcome: Record<string, unknown> | undefined): Pick<ActivitySummary, 'primaryLabel' | 'primaryValue'> {
  if (flags.isClean) return { primaryLabel: 'Space Reclaimed', primaryValue: outcomeBytes(outcome, 'bytesFreed') };
  if (flags.isScan) return { primaryLabel: 'Space Found', primaryValue: outcomeBytes(outcome, 'totalBytes') };
  return { primaryLabel: 'Space Reclaimed', primaryValue: '—' };
}

function countCategories(skippedFileRows: AuditRow[]): number {
  const parents = skippedFileRows.map((r) => (r.sublabel || r.category || '').split('·')[0].trim()).filter(Boolean);
  const unique = new Set(parents).size;
  if (unique > 0) return unique;
  return skippedFileRows.length;
}

function eligibleCategories(args: { flags: Flags; rows: AuditRow[]; outcome?: Record<string, unknown>; intent?: Record<string, unknown> | null }): number {
  const cleaned = args.rows.filter((r) => r.kind === 'Category' && r.outcomeTone === 'cleaned').length;
  const eligible = args.rows.filter((r) => r.kind === 'Category' && r.outcomeTone === 'eligible').length;
  const count = args.flags.isClean ? cleaned : eligible;
  if (count) return count;
  if (args.outcome?.categoriesCount != null) return Number(args.outcome.categoriesCount);
  if (args.intent?.categoriesCount != null) return Number(args.intent.categoriesCount);
  return 0;
}

function eligibleFiles(flags: Flags, rows: AuditRow[], outcome?: Record<string, unknown>): number {
  if (flags.isClean) return Number(outcome?.deletedCount ?? rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'cleaned').length ?? 0);
  const eligible = rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'eligible').length;
  const skipped = rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped').length;
  return Math.max(0, eligible || (Number(outcome?.items ?? 0) - skipped));
}

function pluralize(n: number, one: string, many: string): string { return `${n} ${n === 1 ? one : many}`; }

function summaryCounts(args: { flags: Flags; rows: AuditRow[]; outcome?: Record<string, unknown>; intent?: Record<string, unknown> | null }) {
  const files = eligibleFiles(args.flags, args.rows, args.outcome);
  const cats = eligibleCategories(args);
  const skippedRows = args.rows.filter((r) => r.kind === 'File' && r.outcomeTone === 'skipped');
  const eligible = `${pluralize(files, 'file', 'files')} · ${pluralize(cats, 'category', 'categories')}`;
  const skipped = `${pluralize(skippedRows.length, 'file', 'files')} · ${pluralize(countCategories(skippedRows), 'category', 'categories')}`;
  return { eligible, skipped };
}

function emptyActivity(): ActivitySummary {
  return { primaryLabel: 'Space Reclaimed', primaryValue: '—', eligible: '—', skipped: '—', outcome: '—', mode: null };
}

function buildActivity(content: string): ActivitySummary {
  const objs = parseLogObjects(content);
  const flags = detectCleanScan(objs);
  const primary = primaryMetric(flags, flags.outcome ?? undefined);
  const rows = parseAuditRows(content, null);
  const counts = summaryCounts({ flags, rows, outcome: flags.outcome ?? undefined, intent: flags.intent });
  const status = String(flags.outcome?.status ?? 'completed');
  const outcome = status === 'completed' ? 'Completed' : status;
  const mode = flags.isClean ? 'Permanent Delete' : null;
  return { ...primary, ...counts, outcome, mode };
}

export function deriveActivitySummary(content: string | null): ActivitySummary {
  if (!content) return emptyActivity();
  try { return buildActivity(content); } catch { return { ...emptyActivity(), outcome: 'Recorded' }; }
}
