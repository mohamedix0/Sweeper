import { formatBytes } from '../../utils/format';
import type { AuditRow, LogMeta } from './logParsers';
import type { SweeperClient } from '../../api/client';

function sizeUnitMult(unit: string): number {
  if (unit === 'B') return 1;
  if (unit === 'KB') return 1024;
  if (unit === 'MB') return 1024 ** 2;
  if (unit === 'GB') return 1024 ** 3;
  return 1024 ** 4;
}

export function sizeFromLabel(size: string): number {
  const m = String(size || '').match(/^([\d.]+)\s*(B|KB|MB|GB|TB)$/i);
  if (!m) return 0;
  const val = parseFloat(m[1]);
  if (!Number.isFinite(val)) return 0;
  const mult = sizeUnitMult(m[2].toUpperCase());
  return val * mult;
}

function groupLabel(parent: string): string {
  if (parent === 'Custom Path (Unknown)') return 'CustomCleanup';
  if (parent.startsWith('Custom Path')) return 'CustomCleanup';
  if (parent.includes('CustomCleanup')) return 'CustomCleanup';
  return parent;
}

function groupKey(args: { parent: string; reason: string }): string {
  return `${args.parent}::${args.reason}`;
}

function defaultParent(row: AuditRow): string {
  if (row.sublabel) return row.sublabel;
  if (row.category) return row.category;
  return 'Skipped';
}

function rowReason(row: AuditRow): string {
  const reason = row.outcome.replace(/^Skipped\s*[·\-–—]?\s*/i, '').trim();
  if (reason) return reason;
  return 'Skipped';
}

export function groupSkippedRows(rows: AuditRow[]) {
  const grouped = new Map<string, { key: string; label: string; reason: string; rows: AuditRow[] }>();
  for (const r of rows) {
    const parent = defaultParent(r);
    const reason = rowReason(r);
    const key = groupKey({ parent, reason });
    const g = grouped.get(key) ?? { key, label: groupLabel(parent), reason, rows: [] };
    g.rows.push(r);
    grouped.set(key, g);
  }
  return Array.from(grouped.values());
}

export function formatGroupBytes(rows: AuditRow[]): string {
  const total = rows.reduce((sum, r) => sum + sizeFromLabel(r.size), 0);
  if (total > 0) return formatBytes({ bytes: String(Math.round(total)) });
  return '\u2014';
}

async function readOne(client: SweeperClient, item: LogMeta): Promise<readonly [string, string] | null> {
  try {
    const log = await client.readLog(item.id);
    return [item.id, log.content] as const;
  } catch {
    return null;
  }
}

function addEntry(map: Record<string, string>, entry: readonly [string, string] | null) {
  if (!entry) return;
  map[entry[0]] = entry[1];
}

export async function readLogMap(client: SweeperClient, items: LogMeta[]) {
  const entries = await Promise.all(items.slice(0, 15).map((item) => readOne(client, item)));
  const map: Record<string, string> = {};
  for (const entry of entries) addEntry(map, entry);
  return map;
}
