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
interface ValueOptions { value: string }

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const MIN_SAFE = BigInt(Number.MIN_SAFE_INTEGER);
const DASH = '\u2014';
const ELLIPSIS = '\u2026';

export function toBigInt(args: ToBigIntOptions): bigint {
  const { value } = args;
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number') return BigInt(Math.trunc(value));
  const trimmed = value.trim();
  if (!/^-?\d+$/.test(trimmed)) throw new Error('Invalid integer string');
  return BigInt(trimmed);
}

export function sumDecimalStrings(args: SumOptions): bigint {
  return args.values.reduce<bigint>((acc, value) => acc + toBigInt({ value }), 0n);
}

function scaleBytes(value: bigint): { unitIndex: number; whole: bigint; frac: bigint } {
  let n = value;
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
  if (parsed < 0n) return formatScaled({ negative: true, ...scaleBytes(-parsed) });
  if (parsed === 0n) return '0 B';
  return formatScaled({ negative: false, ...scaleBytes(parsed) });
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

function relativeFromHours(hours: number): string {
  if (hours < 24) return `${hours}h ago`;
  return relativeFromDays(Math.floor(hours / 24));
}

function relativeFromMinutes(minutes: number): string {
  if (minutes < 2) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  return relativeFromHours(Math.floor(minutes / 60));
}

function relativeFromMonths(days: number): string {
  if (days >= 365) return `${Math.floor(days / 365)}y ago`;
  return `${Math.floor(days / 30)}mo ago`;
}

function relativeFromDays(days: number): string {
  if (days < 30) return `${days}d ago`;
  return relativeFromMonths(days);
}

export function relativeAge(args: RelativeAgeOptions): string {
  const nowMs = args.nowMs ?? Date.now();
  if (args.secs == null) return DASH;
  const ms = safeMsFromSecs({ secs: String(args.secs) });
  if (ms == null) return DASH;
  const diff = nowMs - ms;
  if (diff <= 0) return 'just now';
  return relativeFromMinutes(Math.floor(diff / 60000));
}

export function pluralize(args: PluralizeOptions): string {
  if (args.count === 1) return `${args.count} ${args.singular}`;
  const plural = args.plural ?? `${args.singular}s`;
  return `${args.count} ${plural}`;
}

function splitNormalizedPath(path: string): { prefix: string; rest: string } {
  if (/^\\\\\?\\UNC\\/i.test(path)) return { prefix: path.slice(0, 8), rest: path.slice(8) };
  if (/^\\\\\?\\/i.test(path)) return { prefix: path.slice(0, 4), rest: path.slice(4) };
  if (path.startsWith('\\\\')) return { prefix: '\\\\', rest: path.slice(2) };
  return { prefix: '', rest: path };
}

export function normalizeDisplayPath(args: PathOptions): string {
  const normalized = args.p.replace(/\//g, '\\');
  const split = splitNormalizedPath(normalized);
  return split.prefix + split.rest.replace(/\\+/g, '\\');
}

function normalizeSlashes(args: ValueOptions): string {
  return args.value.replace(/\\{2,}/g, (m, offset, str) => {
    if (!str.startsWith('\\\\')) return '\\';
    if (offset === 0) return '\\\\';
    return '\\';
  });
}

function joinMiddle(args: JoinMiddleOptions): string {
  const cleanPrefix = args.prefix.replace(/\\+$/, '\\');
  const tailStr = args.tail.join('\\');
  if (!tailStr) return `${cleanPrefix}${ELLIPSIS}`;
  return `${cleanPrefix}${ELLIPSIS}\\${tailStr}`;
}

function usersPrefix(parts: string[]): string[] | null {
  const first = parts[1] ?? '';
  if (first.toLowerCase() !== 'users') return null;
  if (parts.length < 3) return null;
  return parts.slice(0, 3);
}

function drivePrefix(parts: string[]): string[] {
  return usersPrefix(parts) ?? [parts[0]];
}

function uncPrefix(parts: string[]): string[] {
  return parts.slice(0, Math.min(4, parts.length));
}

function splitPathPrefix(args: PrefixSplitOptions): PrefixInfo {
  const isUnc = args.path.startsWith('\\\\');
  if (isUnc) return { isUnc, hasDrive: false, prefixParts: uncPrefix(args.parts) };
  const hasDrive = /^[A-Za-z]:$/.test(args.parts[0] || '');
  if (hasDrive) return { isUnc: false, hasDrive: true, prefixParts: drivePrefix(args.parts) };
  return { isUnc: false, hasDrive: false, prefixParts: [args.parts[0]] };
}

function prefixStr(parts: string[]): string {
  return `${parts.join('\\')}\\`;
}

function fitsLength(args: { candidate: string; maxLen: number }): boolean {
  return args.candidate.length <= args.maxLen;
}

function tryTailCount(args: TryPrefixOptions & { tailCount: number }): string | null {
  const tailParts = args.parts.slice(-args.tailCount);
  if (args.prefixParts.length + tailParts.length > args.parts.length) return null;
  const joined = joinMiddle({ prefix: prefixStr(args.prefixParts), tail: tailParts });
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
  const joined = joinMiddle({ prefix: prefixStr(args.prefixParts), tail: [args.leaf] });
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
  const candidates = tailCounts(args).map((tailCount) => tryTailCount({ ...args, tailCount }));
  const found = candidates.find((candidate): candidate is string => candidate != null);
  if (found) return found;
  return tryLeafOnly(args);
}

function leafParts(args: LeafTruncateOptions): { core: string; ext: string } {
  const extMatch = args.leaf.match(/(\.[^.\\]{1,10})$/);
  const ext = extMatch?.[1] ?? '';
  if (ext === '') return { core: args.leaf, ext };
  return { core: args.leaf.slice(0, -ext.length), ext };
}

function truncatedLeaf(args: LeafTruncateOptions): string | null {
  const budget = args.maxLen - args.prefixForLeaf.length;
  if (budget < 8) return null;
  const parts = leafParts(args);
  const tailKeep = Math.min(parts.ext.length + 4, Math.max(3, Math.floor(budget / 2)));
  const headKeep = Math.max(2, budget - tailKeep - 1);
  const middle = `${parts.core.slice(0, Math.max(1, headKeep))}${ELLIPSIS}${(parts.core + parts.ext).slice(-Math.max(1, tailKeep))}`;
  const candidate = normalizeSlashes({ value: `${args.prefixForLeaf}${middle}` });
  if (candidate.length > args.maxLen) return null;
  return candidate;
}

function minimalPrefix(args: PrefixInfo & { parts: string[] }): string[] {
  if (args.hasDrive) return [args.parts[0]];
  if (args.isUnc) return args.parts.slice(0, Math.min(4, args.parts.length));
  return [args.parts[0]];
}

function leafOnlyCandidate(args: { minimalPrefixParts: string[]; leaf: string; maxLen: number }): string | null {
  if (!args.leaf) return null;
  const joined = joinMiddle({ prefix: prefixStr(args.minimalPrefixParts), tail: [args.leaf] });
  const candidate = normalizeSlashes({ value: joined });
  if (candidate.length > args.maxLen) return null;
  return candidate;
}

function keepEmptySegment(args: { path: string; index: number }): boolean {
  if (args.index === 0) return true;
  if (args.index === 1) return args.path.startsWith('\\\\');
  return false;
}

function pathParts(path: string): string[] {
  return path.split('\\').filter((seg, index) => {
    if (seg.length > 0) return true;
    return keepEmptySegment({ path, index });
  });
}

function truncatedFallback(args: { minPrefix: string[]; leaf: string; maxLen: number }): string | null {
  if (!args.leaf) return null;
  return truncatedLeaf({ prefixForLeaf: `${prefixStr(args.minPrefix)}${ELLIPSIS}\\`, leaf: args.leaf, maxLen: args.maxLen });
}

function middleTruncateFallback(args: { p: string; parts: string[]; leaf: string; info: PrefixInfo; maxLen: number }): string {
  const minPrefix = minimalPrefix({ ...args.info, parts: args.parts });
  const minimal = tryWithPrefix({ parts: args.parts, prefixParts: minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (minimal) return minimal;
  const leafOnly = leafOnlyCandidate({ minimalPrefixParts: minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (leafOnly) return leafOnly;
  const truncated = truncatedFallback({ minPrefix, leaf: args.leaf, maxLen: args.maxLen });
  if (truncated) return truncated;
  return `${ELLIPSIS}${args.p.slice(-(args.maxLen - 1))}`;
}

export function middleTruncatePath(args: TruncateOptions): string {
  const maxLen = args.maxLen ?? 38;
  const p = normalizeDisplayPath({ p: args.path });
  if (p.length <= maxLen) return p;
  const parts = pathParts(p);
  if (parts.length === 0) return p.slice(0, maxLen);
  const leaf = parts[parts.length - 1] || '';
  const info = splitPathPrefix({ path: p, parts });
  const preferred = tryWithPrefix({ parts, prefixParts: info.prefixParts, leaf, maxLen });
  if (preferred) return preferred;
  return middleTruncateFallback({ p, parts, leaf, info, maxLen });
}
