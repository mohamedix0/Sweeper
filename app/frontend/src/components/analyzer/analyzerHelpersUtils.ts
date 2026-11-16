import type { AnalyzerNodeDto } from '../../contract/bindings';
import { normalizeDisplayPath } from '../../utils/format';

export function isVhdxPath(args: { path: string }): boolean {
  return args.path.toLowerCase().endsWith('.vhdx');
}

function isDriveRootPath(args: { path: string }): boolean {
  const trimmed = args.path.trim();
  if (/^[A-Za-z]:\\?$/.test(trimmed)) return true;
  const normalized = normalizeDisplayPath({ p: args.path });
  return /^[A-Za-z]:\\$/.test(normalized);
}

export function isDriveRoot(args: { node: AnalyzerNodeDto }): boolean {
  return isDriveRootPath({ path: args.node.path });
}

export function normalizedAnalyzerPath(args: { path: string }): string {
  return normalizeDisplayPath({ p: args.path }).replace(/[\\/]+$/, '').toLowerCase();
}

export function nodeTypeTag(args: { node: AnalyzerNodeDto }): string | null {
  if (isVhdxPath({ path: args.node.path })) return 'Virtual Disk';
  if (args.node.isLinkPoint) return 'Symlink';
  if (args.node.isCloudPlaceholder) return 'Cloud';
  return null;
}

export const ALLOC_COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#8b5cf6', '#ef4444', '#64748b'];
export const BROWSE_PATHS = ['C:\\', 'C:\\Users', 'C:\\Windows\\Temp'];

interface ShareRatioArgs { nodeBytes: bigint; rootTotalBytes: bigint }
interface SharePercentArgs { share: number }

function hasShareRatio(args: ShareRatioArgs): boolean {
  if (args.nodeBytes <= 0n) return false;
  return args.rootTotalBytes > 0n;
}

function shareRatio(args: ShareRatioArgs): number {
  if (!hasShareRatio(args)) return 0;
  return Number(args.nodeBytes) / Number(args.rootTotalBytes);
}

export function shareBarFillPx(args: ShareRatioArgs): number {
  const ratio = shareRatio(args);
  const pct = ratio * 100;
  if (pct < 0.1) return 0;
  const width = Math.round(Math.sqrt(Math.max(0, ratio)) * 48);
  return Math.max(2, Math.min(48, width));
}

export function formatSharePercent(args: SharePercentArgs): string {
  if (args.share <= 0) return '0%';
  if (args.share < 0.1) return '<0.1%';
  const pct1 = Math.floor(args.share * 10) / 10;
  return `${pct1.toFixed(1)}%`;
}

export function escapeVhdxPathForPowerShell(args: { path: string }): string {
  const vhdxPath = normalizeDisplayPath({ p: args.path });
  return `'${vhdxPath.replace(/['\u2018\u2019\u201A\u201B]/g, (q) => q + q)}'`;
}
