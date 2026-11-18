import type { TargetScanResultDto } from '../../contract/bindings';
import { isCustomPathTargetId } from './scanHelpers';

/** Selections at or below this size remain explicit; only large ones are compacted. */
export const COMPACT_SELECTION_THRESHOLD = 128;
/** Sentinels keep the wire representation ID-only (ADR 0002) and backward compatible. */
export const COMPACT_SENTINEL = 'SWSELECT1:';
export const PREPARE_CHUNK_SIZE = 1_000;
export const MAX_COMPACT_IDS = 1_000_000;
/** Small selections keep the historical one-explanation-per-item behaviour. */
export const EXPLANATION_FULL_LIMIT = 64;
export const EXPLANATION_SUMMARY_LIMIT = 16;
export const EXPLANATION_CONCURRENCY = 4;

type Range = { prefix: string; start: number; end: number };
type Descriptor = { ranges: Range[]; explicit: string[] };
type SequenceGroups = { groups: Map<string, number[]>; explicit: string[] };

export type ExplanationTask = { itemId: string; memoKey: string; targetId: string };
export type SelectionIndexEntry = {
  targetId: string;
  targetRisk: TargetScanResultDto['risk'];
  size: string;
  requiresConfirmation: boolean;
};
export type SelectionIndex = Map<string, SelectionIndexEntry>;

export function yieldToEventLoop(): Promise<void> {
  return new Promise((resolve) => { setTimeout(resolve, 0); });
}

/** Scan item IDs are `<target>::<sequence>`; anything else is carried verbatim. */
function idParts(id: string): { prefix: string; seq: number } | null {
  const at = id.lastIndexOf('::');
  if (at <= 0) return null;
  const raw = id.slice(at + 2);
  if (!/^\d+$/.test(raw)) return null;
  const seq = Number(raw);
  if (!Number.isSafeInteger(seq) || seq <= 0) return null;
  return { prefix: id.slice(0, at), seq };
}

function newGroups(): SequenceGroups {
  return { groups: new Map<string, number[]>(), explicit: [] };
}

function collectInto(into: SequenceGroups, ids: Iterable<string>): void {
  for (const id of ids) {
    const parts = idParts(id);
    if (!parts) {
      into.explicit.push(id);
      continue;
    }
    const existing = into.groups.get(parts.prefix);
    if (existing) existing.push(parts.seq);
    else into.groups.set(parts.prefix, [parts.seq]);
  }
}

function descriptorFrom(groups: SequenceGroups): Descriptor {
  const ranges: Range[] = [];
  const prefixes = [...groups.groups.keys()].sort();
  for (const prefix of prefixes) {
    const seqs = groups.groups.get(prefix)!.slice().sort((a, b) => a - b);
    let start = seqs[0];
    let end = start;
    for (let i = 1; i <= seqs.length; i += 1) {
      const next = seqs[i];
      if (next === end + 1) {
        end = next;
        continue;
      }
      ranges.push({ prefix, start, end });
      start = next;
      end = next;
    }
  }
  return { ranges, explicit: groups.explicit };
}

function descriptorToWire(descriptor: Descriptor): string[] {
  return [COMPACT_SENTINEL + JSON.stringify(descriptor)];
}

/** Pure, synchronous form used by tests and by the mock backend's mirror decoder. */
export function buildPreviewSelectionIds(selected: ReadonlySet<string>): string[] {
  if (selected.size <= COMPACT_SELECTION_THRESHOLD) return [...selected];
  const groups = newGroups();
  collectInto(groups, selected);
  return descriptorToWire(descriptorFrom(groups));
}

/**
 * Chunked form used by the preview action. It yields between chunks so the
 * renderer stays responsive and reports determinate progress to the caller.
 */
export async function preparePreviewSelection(
  selected: ReadonlySet<string>,
  onProgress?: (completed: number, total: number) => void,
): Promise<string[]> {
  const total = selected.size;
  if (total <= COMPACT_SELECTION_THRESHOLD) {
    onProgress?.(total, total);
    return [...selected];
  }
  const groups = newGroups();
  const all = [...selected];
  let completed = 0;
  for (let i = 0; i < all.length; i += PREPARE_CHUNK_SIZE) {
    collectInto(groups, all.slice(i, i + PREPARE_CHUNK_SIZE));
    completed = Math.min(i + PREPARE_CHUNK_SIZE, all.length);
    onProgress?.(completed, total);
    await yieldToEventLoop();
  }
  return descriptorToWire(descriptorFrom(groups));
}

/** Mirror of the Rust decoder. Unknown/corrupt descriptors fail closed as explicit IDs. */
export function expandPreviewSelectionIds(ids: string[]): string[] {
  if (ids.length !== 1 || !ids[0].startsWith(COMPACT_SENTINEL)) return ids;
  try {
    const descriptor = JSON.parse(ids[0].slice(COMPACT_SENTINEL.length)) as Descriptor;
    if (!Array.isArray(descriptor.ranges) || !Array.isArray(descriptor.explicit)) return ids;
    const expanded: string[] = [];
    for (const range of descriptor.ranges) {
      if (typeof range.prefix !== 'string') return ids;
      const start = Number(range.start);
      const end = Number(range.end);
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 1 || end < start) return ids;
      if (end - start + 1 > MAX_COMPACT_IDS - expanded.length) return ids;
      for (let n = start; n <= end; n += 1) expanded.push(`${range.prefix}::${n}`);
    }
    for (const id of descriptor.explicit) {
      if (typeof id !== 'string') return ids;
      expanded.push(id);
    }
    if (expanded.length > MAX_COMPACT_IDS) return ids;
    return expanded;
  } catch {
    return ids;
  }
}

/**
 * Small selections keep one IPC per item (preserving per-item failure notices).
 * Large selections resolve at most one representative per Target, memoised by
 * Target for that Preview generation, with a hard global cap.
 */
export function planExplanationTasks(args: {
  selected: ReadonlySet<string>;
  targetIdFor: (itemId: string) => string | undefined;
}): ExplanationTask[] {
  const tasks: ExplanationTask[] = [];
  if (args.selected.size <= EXPLANATION_FULL_LIMIT) {
    for (const itemId of args.selected) {
      tasks.push({ itemId, memoKey: itemId, targetId: args.targetIdFor(itemId) ?? itemId });
    }
    return tasks;
  }
  const seen = new Set<string>();
  for (const itemId of args.selected) {
    if (tasks.length >= EXPLANATION_SUMMARY_LIMIT) break;
    const targetId = args.targetIdFor(itemId) ?? itemId;
    if (seen.has(targetId)) continue;
    seen.add(targetId);
    tasks.push({ itemId, memoKey: targetId, targetId });
  }
  return tasks;
}

/** Minimal index-based pool: bounded IPC fan-out for explanation resolution. */
export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  worker: (item: T) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  const run = async () => {
    while (next < items.length) {
      const at = next;
      next += 1;
      results[at] = await worker(items[at]);
    }
  };
  const runners = Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, () => run());
  await Promise.all(runners);
  return results;
}

export function buildSelectionIndex(args: { targets: TargetScanResultDto[] }): SelectionIndex {
  const index: SelectionIndex = new Map();
  for (const target of args.targets) {
    const requiresConfirmation = target.risk === 'Unknown' || isCustomPathTargetId({ targetId: target.targetId });
    for (const item of target.items) {
      index.set(item.id, { targetId: target.targetId, targetRisk: target.risk, size: item.size, requiresConfirmation });
    }
  }
  return index;
}

export function selectedBytesFromIndex(args: { index: SelectionIndex; selected: ReadonlySet<string> }): string {
  let total = 0n;
  for (const id of args.selected) {
    const entry = args.index.get(id);
    if (entry) total += BigInt(entry.size);
  }
  return total.toString();
}

export function selectedCategoryCountFromIndex(args: { index: SelectionIndex; selected: ReadonlySet<string> }): number {
  const targets = new Set<string>();
  for (const id of args.selected) {
    const entry = args.index.get(id);
    if (entry) targets.add(entry.targetId);
  }
  return targets.size;
}

export function selectionNeedsConfirmationFromIndex(args: { index: SelectionIndex; selected: ReadonlySet<string> }): boolean {
  for (const id of args.selected) {
    if (args.index.get(id)?.requiresConfirmation) return true;
  }
  return false;
}
