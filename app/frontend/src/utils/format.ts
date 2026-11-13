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
  const whole = n;
  const frac2 = (remainderTimes100 % 100n).toString().padStart(2, '0').slice(0, 2);
  const display = `${whole.toString()}.${frac2}`.replace(/\.00$/, '').replace(/(\.\d)0$/, '$1');
  return `${negative ? '-' : ''}${display} ${units[unitIndex]}`;
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
