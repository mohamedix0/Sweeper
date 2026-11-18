import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import type { PreviewRiskCalloutSource } from '../PreviewRiskCallouts';
import { calloutCopyFor } from '../previewRiskCalloutStrings';
import { isCustomPathTargetId } from './scanHelpers';

/** Shared fallback copy when the Knowledge Base explanation cannot be shown. */
export const EXPLANATION_UNAVAILABLE = 'Explanation unavailable for this item — showing its known identity and risk only.';

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

/**
 * Scan item IDs are `<target>::<sequence>`; anything else is carried verbatim.
 * A noncanonical suffix (for example `007`) is not range-compressible: expanding
 * a range would turn it into `7`, changing the ID the backend must resolve.
 * Such IDs are carried in the descriptor's `explicit` list instead.
 */
function idParts(id: string): { prefix: string; seq: number; canonical: boolean } | null {
  const at = id.lastIndexOf('::');
  if (at <= 0) return null;
  const raw = id.slice(at + 2);
  if (!/^\d+$/.test(raw)) return null;
  const seq = Number(raw);
  if (!Number.isSafeInteger(seq) || seq <= 0) return null;
  return { prefix: id.slice(0, at), seq, canonical: String(seq) === raw };
}

function newGroups(): SequenceGroups {
  return { groups: new Map<string, number[]>(), explicit: [] };
}

function collectInto(into: SequenceGroups, ids: Iterable<string>): void {
  for (const id of ids) {
    const parts = idParts(id);
    if (!parts || !parts.canonical) {
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

/** Decode a single sentinel-prefixed wire value; null means "not our descriptor". */
function decodeSelectionDescriptor(ids: string[]): Descriptor | null {
  if (ids.length !== 1) return null;
  if (!ids[0].startsWith(COMPACT_SENTINEL)) return null;
  const descriptor = JSON.parse(ids[0].slice(COMPACT_SENTINEL.length)) as Descriptor;
  if (!Array.isArray(descriptor.ranges)) return null;
  if (!Array.isArray(descriptor.explicit)) return null;
  return descriptor;
}

/** Validated bounds for one range; null when the range is corrupt or unsafe. */
function rangeBounds(range: Range): { start: number; end: number } | null {
  if (typeof range.prefix !== 'string') return null;
  // The descriptor is parsed from JSON at runtime: quoted numbers and booleans
  // would be coerced by Number(...), so require the original values to be numbers.
  if (typeof range.start !== 'number' || typeof range.end !== 'number') return null;
  const start = range.start;
  const end = range.end;
  if (!Number.isSafeInteger(start)) return null;
  if (!Number.isSafeInteger(end)) return null;
  if (start < 1) return null;
  if (end < start) return null;
  return { start, end };
}

/** Expand all ranges into explicit IDs; null if any range is invalid or oversized. */
function expandRanges(ranges: Range[]): string[] | null {
  const expanded: string[] = [];
  for (const range of ranges) {
    const bounds = rangeBounds(range);
    if (!bounds) return null;
    const span = bounds.end - bounds.start + 1;
    if (span > MAX_COMPACT_IDS - expanded.length) return null;
    for (let n = bounds.start; n <= bounds.end; n += 1) expanded.push(`${range.prefix}::${n}`);
  }
  return expanded;
}

/** Append verbatim explicit IDs; false when any entry is not a string. */
function appendExplicitIds(expanded: string[], explicit: string[]): boolean {
  for (const id of explicit) {
    if (typeof id !== 'string') return false;
    expanded.push(id);
  }
  return true;
}

/** Mirror of the Rust decoder. Unknown/corrupt descriptors fail closed as explicit IDs. */
export function expandPreviewSelectionIds(ids: string[]): string[] {
  try {
    const descriptor = decodeSelectionDescriptor(ids);
    if (!descriptor) return ids;
    const expanded = expandRanges(descriptor.ranges);
    if (!expanded) return ids;
    if (!appendExplicitIds(expanded, descriptor.explicit)) return ids;
    return expanded.length <= MAX_COMPACT_IDS ? expanded : ids;
  } catch {
    return ids;
  }
}

type ExplanationPlanArgs = {
  selected: ReadonlySet<string>;
  targetIdFor: (itemId: string) => string | undefined;
};

/** Small selections: one IPC per item, preserving per-item failure notices. */
function planFullExplanationTasks(args: ExplanationPlanArgs): ExplanationTask[] {
  const tasks: ExplanationTask[] = [];
  for (const itemId of args.selected) {
    tasks.push({ itemId, memoKey: itemId, targetId: args.targetIdFor(itemId) ?? itemId });
  }
  return tasks;
}

/** Large selections: one representative per Target, memoised, with a global cap. */
function planSummaryExplanationTasks(args: ExplanationPlanArgs): ExplanationTask[] {
  const tasks: ExplanationTask[] = [];
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

export function planExplanationTasks(args: ExplanationPlanArgs): ExplanationTask[] {
  return args.selected.size <= EXPLANATION_FULL_LIMIT
    ? planFullExplanationTasks(args)
    : planSummaryExplanationTasks(args);
}

/** Minimal index-based pool: bounded IPC fan-out for explanation resolution. */
function fillSkippedResults<T, R>(
  items: readonly T[],
  from: number,
  results: R[],
  onSkip?: (item: T) => R,
): void {
  for (let i = from; i < items.length; i += 1) {
    if (onSkip) results[i] = onSkip(items[i]);
  }
}

export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  worker: (item: T) => Promise<R>,
  options?: { shouldStop?: () => boolean; onSkip?: (item: T) => R },
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  const run = async () => {
    while (next < items.length) {
      if (options?.shouldStop?.()) {
        fillSkippedResults(items, next, results, options?.onSkip);
        return;
      }
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

export type ItemRiskContext = { target: TargetScanResultDto; item: ScanItemDto };

/**
 * Every selected item gets a known-risk source straight from the Scan result —
 * no IPC and no target cap. This is what guarantees one warning per elevated-
 * Risk selected item (including Caution/Custom Path targets beyond the bounded
 * explanation summary) and lets Clean be gated on complete coverage.
 */
export function buildKnownRiskSources(args: {
  selection: ReadonlySet<string>;
  itemContext: ReadonlyMap<string, ItemRiskContext>;
}): PreviewRiskCalloutSource[] {
  const sources: PreviewRiskCalloutSource[] = [];
  for (const itemId of args.selection) {
    const ctx = args.itemContext.get(itemId);
    sources.push({
      itemId,
      title: ctx?.item.path ?? itemId,
      description: EXPLANATION_UNAVAILABLE,
      risk: ctx?.target.risk ?? 'Unknown',
      targetId: ctx?.target.targetId ?? null,
    });
  }
  return sources;
}

/** Knowledge Base explanations replace the known-risk source for the same item. */
export function mergeRiskSources(
  known: readonly PreviewRiskCalloutSource[],
  resolved: readonly PreviewRiskCalloutSource[],
): PreviewRiskCalloutSource[] {
  const byId = new Map(known.map((source) => [source.itemId, source]));
  for (const source of resolved) {
    const previous = byId.get(source.itemId);
    // An explanation must never hide a warning: keep the known elevated Risk
    // if the Knowledge Base result reports no callout-worthy Risk for it.
    const risk = previous != null && calloutCopyFor(previous.risk) !== null && calloutCopyFor(source.risk) === null
      ? previous.risk
      : source.risk;
    // The known path is the per-item identity: an explanation for a shared
    // Target can repeat the same title for several items, which would make
    // their warnings indistinguishable. Use it only to enrich the description.
    byId.set(source.itemId, { ...source, title: previous?.title ?? source.title, risk });
  }
  return [...byId.values()];
}

export function riskCoverageComplete(args: {
  selection: ReadonlySet<string>;
  sources: readonly PreviewRiskCalloutSource[];
}): boolean {
  const covered = new Set(args.sources.map((source) => source.itemId));
  for (const id of args.selection) if (!covered.has(id)) return false;
  return true;
}
