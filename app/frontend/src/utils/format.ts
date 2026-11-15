export function toBigInt(value: string | number | bigint): bigint {
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number') return BigInt(Math.trunc(value));
  const trimmed = value.trim();
  if (!/^-?\d+$/.test(trimmed)) {
    throw new Error(`Invalid integer string: ${value}`);
  }
  return BigInt(trimmed);
}

export function sumDecimalStrings(values: Array<string | bigint | number>): bigint {
  return values.reduce<bigint>((acc, v) => acc + toBigInt(v), 0n);
}

export function formatBytes(bytes: string | bigint | number): string {
  let b: bigint;
  try {
    b = toBigInt(bytes);
  } catch {
    return '0 B';
  }
  const negative = b < 0n;
  let n = negative ? -b : b;
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  if (n === 0n) return '0 B';
  let unitIndex = 0;
  let remainderTimes100 = 0n;
  while (n >= 1024n && unitIndex < units.length - 1) {
    remainderTimes100 = ((n % 1024n) * 100n) / 1024n;
    n = n / 1024n;
    unitIndex += 1;
  }
  if (unitIndex === 0) {
    return `${negative ? '-' : ''}${n.toString()} ${units[unitIndex]}`;
  }
  const unit = units[unitIndex];
  const whole = n;
  const frac2 = (remainderTimes100 % 100n).toString().padStart(2, '0').slice(0, 2);
  const display = (unit === 'GB' || unit === 'TB')
    ? `${whole.toString()}.${frac2}`
    : `${whole.toString()}.${frac2}`.replace(/\.00$/, '').replace(/(\.\d)0$/, '$1');
  return `${negative ? '-' : ''}${display} ${unit}`;
}

export function formatUnixSeconds(secs: string | null | undefined): string {
  if (!secs) return '—';
  try {
    const msBig = toBigInt(secs) * 1000n;
    const max = BigInt(Number.MAX_SAFE_INTEGER);
    const min = BigInt(Number.MIN_SAFE_INTEGER);
    if (msBig > max || msBig < min) return secs;
    const ms = Number(msBig);
    if (!Number.isFinite(ms)) return secs;
    const d = new Date(ms);
    if (Number.isNaN(d.getTime())) return secs;
    return d.toLocaleString();
  } catch {
    return String(secs);
  }
}

export function relativeAge(secs: string | number | bigint | null | undefined, nowMs: number = Date.now()): string {
  if (!secs) return '—';
  try {
    const msBig = toBigInt(secs) * 1000n;
    const max = BigInt(Number.MAX_SAFE_INTEGER);
    const min = BigInt(Number.MIN_SAFE_INTEGER);
    if (msBig > max || msBig < min) return '—';
    const then = Number(msBig);
    if (!Number.isFinite(then)) return '—';
    const diff = nowMs - then;
    if (diff <= 0) return 'just now';
    const minutes = Math.floor(diff / 60000);
    if (minutes < 2) return 'just now';
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days}d ago`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months}mo ago`;
    const years = Math.floor(days / 365);
    return `${years}y ago`;
  } catch {
    return '—';
  }
}

export function pluralize(count: number, singular: string, plural = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

export function normalizeDisplayPath(p: string): string {
  return p.replace(/\//g, '\\').replace(/\\+/g, '\\');
}

export function middleTruncatePath(path: string, maxLen = 38): string {
  const p = normalizeDisplayPath(path);
  if (p.length <= maxLen) return p;

  const parts = p.split('\\').filter((seg, i) => seg.length > 0 || i === 0);
  if (parts.length === 0) return p.slice(0, maxLen);

  const hasDrive = /^[A-Za-z]:$/.test(parts[0] || '');
  const isUnc = p.startsWith('\\\\');
  const leaf = parts[parts.length - 1] || '';

  let prefixParts: string[] = [];
  if (isUnc) {
    prefixParts = parts.slice(0, Math.min(4, parts.length));
  } else if (hasDrive) {
    const lowerUsers = (parts[1] || '').toLowerCase() === 'users';
    if (lowerUsers && parts.length >= 3) {
      prefixParts = parts.slice(0, 3); // C:\Users\Alice
    } else {
      prefixParts = [parts[0]]; // C:
    }
  } else {
    prefixParts = [parts[0]];
  }

  const normalizeSlashes = (s: string) =>
    s.replace(/\\{2,}/g, (m, offset, str) => (str.startsWith('\\\\') && offset === 0 ? '\\\\' : '\\'));

  const joinMiddle = (prefix: string, tail: string[]) => {
    const cleanPrefix = prefix.replace(/\\+$/, '\\');
    const tailStr = tail.join('\\');
    if (!tailStr) return `${cleanPrefix}…`;
    return `${cleanPrefix}…\\${tailStr}`;
  };

  const tryWithPrefix = (prefix: string[]) => {
    const prefixStr = `${prefix.join('\\')}\\`;
    const maxTail = Math.min(3, parts.length - prefix.length);
    for (let tailCount = Math.min(2, maxTail); tailCount >= 1; tailCount--) {
      const tailParts = parts.slice(-tailCount);
      if (prefix.length + tailParts.length > parts.length) continue;
      const candidate = normalizeSlashes(joinMiddle(prefixStr, tailParts));
      if (candidate.length <= maxLen) return candidate;
    }
    if (leaf && prefix.length < parts.length) {
      const candidate = normalizeSlashes(joinMiddle(prefixStr, [leaf]));
      if (candidate.length <= maxLen) return candidate;
    }
    return null;
  };

  const preferred = tryWithPrefix(prefixParts);
  if (preferred) return preferred;

  const minimalPrefixParts = hasDrive ? [parts[0]] : (isUnc ? parts.slice(0, 2) : [parts[0]]);
  const minimal = tryWithPrefix(minimalPrefixParts);
  if (minimal) return minimal;

  // Preserve full leaf name when possible by trimming more of the middle/prefix context.
  const minimalPrefixStr = `${minimalPrefixParts.join('\\')}\\`;
  const leafOnly = leaf ? normalizeSlashes(joinMiddle(minimalPrefixStr, [leaf])) : null;
  if (leafOnly && leafOnly.length <= maxLen) return leafOnly;

  // Last resort: middle-truncate a very long leaf but keep its start/end (and extension) intact.
  if (leaf) {
    const prefixForLeaf = `${minimalPrefixStr}…\\`;
    const budget = maxLen - prefixForLeaf.length;
    if (budget >= 8) {
      const extMatch = leaf.match(/(\.[^.\\]{1,10})$/);
      const ext = extMatch ? extMatch[1] : '';
      const core = ext ? leaf.slice(0, -ext.length) : leaf;
      const tailKeep = Math.min(ext.length + 4, Math.max(3, Math.floor(budget / 2)));
      const headKeep = Math.max(2, budget - tailKeep - 1);
      const middleLeaf = `${core.slice(0, Math.max(1, headKeep))}…${(core + ext).slice(-Math.max(1, tailKeep))}`;
      const candidate = normalizeSlashes(`${prefixForLeaf}${middleLeaf}`);
      if (candidate.length <= maxLen) return candidate;
    }
  }

  return `…${p.slice(-(maxLen - 1))}`;
}
