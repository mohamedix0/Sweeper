type IntLike = string | number | bigint;
interface ToBigIntOptions { value: IntLike }
interface BytesOptions { bytes: IntLike }
interface SumOptions { values: Array<IntLike> }
interface UnixSecsOptions { secs: string | null | undefined }
interface RelativeAgeOptions { secs: IntLike | null | undefined; nowMs?: number }
interface PluralizeOptions { count: number; singular: string; plural?: string }
interface PathOptions { p: string }
interface TruncateOptions { path: string; maxLen?: number }
interface PrefixSplitOptions { path: string; parts: string[] }
interface PrefixInfo { isUnc: boolean; hasDrive: boolean; prefixParts: string[] }
interface JoinMiddleOptions { prefix: string; tail: string[] }
interface TryPrefixOptions { parts: string[]; prefixParts: string[]; leaf: string; maxLen: number }
interface LeafTruncateOptions { prefixForLeaf: string; leaf: string; maxLen: number }

// Small value objects for the concepts that flow through this module. Passing
// these instead of bare primitives makes each argument's meaning explicit and
// keeps the module's argument surface strongly typed.
interface TextValue { readonly value: string }
interface TakeText { readonly value: string; readonly count: number }
interface Glyphs { readonly clusters: string[]; readonly length: number }
interface ByteMagnitude { readonly value: bigint }
interface TimeAmount { readonly amount: number }
interface PathSegments { readonly parts: string[] }

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE = BigInt(Number.MIN_SAFE_INTEGER);
const DASH = '\u2014';
const ELLIPSIS = '\u2026';

interface GraphemeSegmenter {
  segment(input: string): Iterable<{ segment: string }>;
}
type GraphemeSegmenterCtor = new (
  locales?: string | string[],
  options?: { granularity?: 'grapheme' | 'word' | 'sentence' },
) => GraphemeSegmenter;

const SegmenterCtor = (Intl as unknown as { Segmenter?: GraphemeSegmenterCtor }).Segmenter;

// Display text is measured as a sequence of grapheme clusters so an emoji or
// combining sequence is never cut in half. Array.from is a surrogate-safe
// fallback for runtimes without Segmenter (it splits by code point).
function glyphsOf(args: TextValue): Glyphs {
  const value = args.value;
  if (SegmenterCtor) {
    const segmenter = new SegmenterCtor(undefined, { granularity: 'grapheme' });
    const clusters: string[] = [];
    for (const part of segmenter.segment(value)) clusters.push(part.segment);
    return { clusters, length: clusters.length };
  }
  const clusters = Array.from(value);
  return { clusters, length: clusters.length };
}

function glyphLength(args: TextValue): number {
  return glyphsOf(args).length;
}

function takeGlyphs(args: TakeText): string {
  return glyphsOf({ value: args.value }).clusters.slice(0, args.count).join('');
}

function takeLastGlyphs(args: TakeText): string {
  return glyphsOf({ value: args.value }).clusters.slice(-args.count).join('');
}

export function toBigInt(args: ToBigIntOptions): bigint {
  const { value } = args;
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number') return BigInt(Math.trunc(value));
  const trimmed = value.trim();
  if (!/^-?\d+$/.test(trimmed)) throw new Error('Invalid integer string');
  return BigInt(trimmed);
}

export function sumDecimalStrings(args: SumOptions): bigint {
  let total = 0n;
  for (const value of args.values) total += toBigInt({ value });
  return total;
}

function scaleBytes(args: ByteMagnitude): { unitIndex: number; whole: bigint; frac: bigint } {
  let n = args.value;
  let unitIndex = 0;
  let frac = 0n;
  while (n >= 1024n && unitIndex < UNITS.length - 1) {
    frac = ((n % 1024n) * 100n) / 1024n;
    n /= 1024n;
    unitIndex += 1;
  }
  return { unitIndex, whole: n, frac };
}

function normalizeDisplay(args: { unit: string; whole: bigint; frac: bigint }): string {
  const display = `${args.whole.toString()}.${args.frac.toString().padStart(2, '0').slice(0, 2)}`;
  if (args.unit === 'GB') return display;
  if (args.unit === 'TB') return display;
  return display.replace(/\.00$/, '').replace(/(\.\d)0$/, '$1');
}

function signPrefix(args: { negative: boolean }): string {
  if (args.negative) return '-';
  return '';
}

function formatScaled(args: { negative: boolean; unitIndex: number; whole: bigint; frac: bigint }): string {
  const sign = signPrefix(args);
  const unit = UNITS[args.unitIndex];
  if (args.unitIndex === 0) return `${sign}${args.whole.toString()} ${unit}`;
  return `${sign}${normalizeDisplay({ unit, whole: args.whole, frac: args.frac })} ${unit}`;
}

export function formatBytes(args: BytesOptions): string {
  let parsed: bigint | null = null;
  try { parsed = toBigInt({ value: args.bytes }); } catch { parsed = null; }
  if (parsed == null) return '0 B';
  if (parsed < 0n) return formatScaled({ negative: true, ...scaleBytes({ value: -parsed }) });
  if (parsed === 0n) return '0 B';
  return formatScaled({ negative: false, ...scaleBytes({ value: parsed }) });
}

function safeMsFromSecs(args: UnixSecsOptions): number | null {
  if (args.secs == null) return null;
  let msBig: bigint | null = null;
  try { msBig = toBigInt({ value: args.secs }) * 1000n; } catch { msBig = null; }
  if (msBig == null) return null;
  if (msBig > MAX_SAFE) return null;
  if (msBig < MIN_SAFE) return null;
  const ms = Number(msBig);
  if (!Number.isFinite(ms)) return null;
  return ms;
}

export function formatUnixSeconds(args: UnixSecsOptions): string {
  if (args.secs == null) return DASH;
  const ms = safeMsFromSecs(args);
  if (ms == null) return String(args.secs);
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return String(args.secs);
  return d.toLocaleString();
}

function relativeFromHours(args: TimeAmount): string {
  if (args.amount < 24) return `${args.amount}h ago`;
  return relativeFromDays({ amount: Math.floor(args.amount / 24) });
}

function relativeFromMinutes(args: TimeAmount): string {
  if (args.amount < 2) return 'just now';
  if (args.amount < 60) return `${args.amount}m ago`;
  return relativeFromHours({ amount: Math.floor(args.amount / 60) });
}

function relativeFromMonths(args: TimeAmount): string {
  if (args.amount >= 365) return `${Math.floor(args.amount / 365)}y ago`;
  return `${Math.floor(args.amount / 30)}mo ago`;
}

function relativeFromDays(args: TimeAmount): string {
  if (args.amount < 30) return `${args.amount}d ago`;
  return relativeFromMonths(args);
}

export function relativeAge(args: RelativeAgeOptions): string {
  const nowMs = args.nowMs ?? Date.now();
  if (args.secs == null) return DASH;
  const ms = safeMsFromSecs({ secs: String(args.secs) });
  if (ms == null) return DASH;
  const diff = nowMs - ms;
  if (diff <= 0) return 'just now';
  return relativeFromMinutes({ amount: Math.floor(diff / 60000) });
}

export function pluralize(args: PluralizeOptions): string {
  if (args.count === 1) return `${args.count} ${args.singular}`;
  const plural = args.plural ?? `${args.singular}s`;
  return `${args.count} ${plural}`;
}

function splitNormalizedPath(args: TextValue): { prefix: string; rest: string } {
  const path = args.value;
  if (/^\\\\\?\\UNC\\/i.test(path)) return { prefix: path.slice(0, 8), rest: path.slice(8) };
  if (/^\\\\\?\\/i.test(path)) return { prefix: path.slice(0, 4), rest: path.slice(4) };
  if (path.startsWith('\\\\')) return { prefix: '\\\\', rest: path.slice(2) };
  return { prefix: '', rest: path };
}

export function normalizeDisplayPath(args: PathOptions): string {
  const normalized = args.p.replace(/\//g, '\\');
  const split = splitNormalizedPath({ value: normalized });
  return split.prefix + split.rest.replace(/\\+/g, '\\');
}

function normalizeSlashes(args: TextValue): string {
  const value = args.value;
  const leading = /^\\{2,}/.exec(value)?.[0] ?? '';
  const rest = value.slice(leading.length).replace(/\\{2,}/g, '\\');
  return (leading ? '\\\\' : '') + rest;
}

function joinMiddle(args: JoinMiddleOptions): string {
  const cleanPrefix = args.prefix.replace(/\\+$/, '\\');
  const tailStr = args.tail.join('\\');
  if (!tailStr) return `${cleanPrefix}${ELLIPSIS}`;
  return `${cleanPrefix}${ELLIPSIS}\\${tailStr}`;
}

function usersPrefix(args: PathSegments): string[] | null {
  const first = args.parts[1] ?? '';
  if (first.toLowerCase() !== 'users') return null;
  if (args.parts.length < 3) return null;
  return args.parts.slice(0, 3);
}

function drivePrefix(args: PathSegments): string[] {
  return usersPrefix(args) ?? [args.parts[0]];
}

function uncPrefix(args: PathSegments): string[] {
  return args.parts.slice(0, Math.min(4, args.parts.length));
}

function splitPathPrefix(args: PrefixSplitOptions): PrefixInfo {
  const isUnc = args.path.startsWith('\\\\');
  if (isUnc) return { isUnc, hasDrive: false, prefixParts: uncPrefix({ parts: args.parts }) };
  const hasDrive = /^[A-Za-z]:$/.test(args.parts[0] || '');
  if (hasDrive) return { isUnc: false, hasDrive: true, prefixParts: drivePrefix({ parts: args.parts }) };
  return { isUnc: false, hasDrive: false, prefixParts: [args.parts[0]] };
}

function prefixStr(args: PathSegments): string {
  return `${args.parts.join('\\')}\\`;
}

function fitsLength(args: { candidate: string; maxLen: number }): boolean {
  return glyphLength({ value: args.candidate }) <= args.maxLen;
}

function tryTailCount(args: TryPrefixOptions & { tailCount: number }): string | null {
  const tailParts = args.parts.slice(-args.tailCount);
  if (args.prefixParts.length + tailParts.length > args.parts.length) return null;
  const joined = joinMiddle({ prefix: prefixStr({ parts: args.prefixParts }), tail: tailParts });
  const candidate = normalizeSlashes({ value: joined });
  if (fitsLength({ candidate, maxLen: args.maxLen })) return candidate;
  return null;
}

function hasLeafRoom(args: TryPrefixOptions): boolean {
  if (!args.leaf) return false;
  return args.prefixParts.length < args.parts.length;
}

function tryLeafOnly(args: TryPrefixOptions): string | null {
  if (!hasLeafRoom(args)) return null;
  const joined = joinMiddle({ prefix: prefixStr({ parts: args.prefixParts }), tail: [args.leaf] });
  const candidate = normalizeSlashes({ value: joined });
  if (fitsLength({ candidate, maxLen: args.maxLen })) return candidate;
  return null;
}

function tailCounts(args: TryPrefixOptions): number[] {
  const maxTail = Math.min(3, args.parts.length - args.prefixParts.length);
  const firstTail = Math.min(2, maxTail);
  const counts: number[] = [];
  for (let tailCount = firstTail; tailCount >= 1; tailCount -= 1) counts.push(tailCount);
  return counts;
}

function tryWithPrefix(args: TryPrefixOptions): string | null {
  for (const tailCount of tailCounts(args)) {
    const candidate = tryTailCount({ ...args, tailCount });
    if (candidate != null) return candidate;
  }
  return tryLeafOnly(args);
}

function leafParts(args: LeafTruncateOptions): { core: string; ext: string } {
  const extMatch = args.leaf.match(/(\.[^.\\]{1,10})$/);
  const ext = extMatch?.[1] ?? '';
  if (ext === '') return { core: args.leaf, ext };
  // Slice by code-unit length rather than subtracting independently measured
  // grapheme counts. A grapheme with the Unicode Prepend property (e.g. U+0600)
  // clusters with the following '.', so a count-based split drops it and
  // corrupts the extension boundary. A direct slice keeps core + ext === leaf.
  return { core: args.leaf.slice(0, args.leaf.length - ext.length), ext };
}

function truncatedLeaf(args: LeafTruncateOptions): string | null {
  const budget = args.maxLen - glyphLength({ value: args.prefixForLeaf });
  if (budget < 8) return null;
  const parts = leafParts(args);
  const tailKeep = Math.min(glyphLength({ value: parts.ext }) + 4, Math.max(3, Math.floor(budget / 2)));
  const headKeep = Math.max(2, budget - tailKeep - 1);
  // Select both head and tail from the ORIGINAL leaf so the grapheme at the
  // extension boundary (e.g. a Prepend character before '.') is never
  // reconstructed from a count-derived core that may have lost it.
  const middle = `${takeGlyphs({ value: args.leaf, count: Math.max(1, headKeep) })}${ELLIPSIS}${takeLastGlyphs({ value: args.leaf, count: Math.max(1, tailKeep) })}`;
  const candidate = normalizeSlashes({ value: `${args.prefixForLeaf}${middle}` });
  if (glyphLength({ value: candidate }) > args.maxLen) return null;
  return candidate;
}

function minimalPrefix(args: PrefixInfo & PathSegments): string[] {
  if (args.hasDrive) return [args.parts[0]];
  if (args.isUnc) return args.parts.slice(0, Math.min(4, args.parts.length));
  return [args.parts[0]];
}

function leafOnlyCandidate(args: { minimalPrefixParts: string[]; leaf: string; maxLen: number }): string | null {
  if (!args.leaf) return null;
  const joined = joinMiddle({ prefix: prefixStr({ parts: args.minimalPrefixParts }), tail: [args.leaf] });
  const candidate = normalizeSlashes({ value: joined });
  if (glyphLength({ value: candidate }) > args.maxLen) return null;
  return candidate;
}

function keepEmptySegment(args: { path: string; index: number }): boolean {
  if (args.index === 0) return true;
  if (args.index === 1) return args.path.startsWith('\\\\');
  return false;
}

function pathParts(args: TextValue): string[] {
  const parts: string[] = [];
  const segments = args.value.split('\\');
  for (let index = 0; index < segments.length; index += 1) {
    const seg = segments[index];
    if (seg.length > 0 || keepEmptySegment({ path: args.value, index })) parts.push(seg);
  }
  return parts;
}

function truncatedFallback(args: { minPrefix: string[]; leaf: string; maxLen: number }): string | null {
  if (!args.leaf) return null;
  return truncatedLeaf({ prefixForLeaf: `${prefixStr({ parts: args.minPrefix })}${ELLIPSIS}\\`, leaf: args.leaf, maxLen: args.maxLen });
}

function middleTruncateFallback(args: { p: string; parts: string[]; leaf: string; info: PrefixInfo; maxLen: number }): string {
  const minPrefix = minimalPrefix({ ...args.info, parts: args.parts });
  const minimal = tryWithPrefix({ parts: args.parts, prefixParts: minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (minimal) return minimal;
  const leafOnly = leafOnlyCandidate({ minimalPrefixParts: minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (leafOnly) return leafOnly;
  const truncated = truncatedFallback({ minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (truncated) return truncated;
  return `${ELLIPSIS}${takeLastGlyphs({ value: args.p, count: args.maxLen - 1 })}`;
}

export function middleTruncatePath(args: TruncateOptions): string {
  const maxLen = args.maxLen ?? 38;
  const p = normalizeDisplayPath({ p: args.path });
  if (glyphLength({ value: p }) <= maxLen) return p;
  const parts = pathParts({ value: p });
  if (parts.length === 0) return takeGlyphs({ value: p, count: maxLen });
  const leaf = parts[parts.length - 1] || '';
  const info = splitPathPrefix({ path: p, parts });
  const preferred = tryWithPrefix({ parts, prefixParts: info.prefixParts, leaf, maxLen });
  if (preferred) return preferred;
  return middleTruncateFallback({ p, parts, leaf, info, maxLen });
}
