import { normalizeDisplayPath, pluralize, relativeAge } from '../utils/format';

export type ExplainState = { itemId: string; title: string; description: string; risk: import('../api/client').RiskDto } | null;
export type PreviewResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  skippedCount?: string;
  skippedReasons?: string[];
  issuedAtMs: number;
};

export const SESSION_TTL_MS = 15 * 60 * 1000;
export const TYPED_CONFIRMATION_PHRASE = 'DELETE';

export async function copyText(args: { text: string }) {
  try {
    await navigator.clipboard?.writeText(args.text);
  } catch {
    /* ignore */
  }
}

function folderForPath(args: { clean: string; idx: number }): string {
  if (args.idx > 2) return args.clean.slice(0, args.idx);
  return 'C:\\';
}

export function openContainingFolder(args: { path: string }) {
  const clean = normalizeDisplayPath({ p: args.path }).replace(/[\\/]+$/, '');
  const idx = clean.lastIndexOf('\\');
  const folder = folderForPath({ clean, idx });
  try {
    window.open(`file:///${folder.replace(/\\/g, '/')}`, '_blank', 'noopener,noreferrer');
  } catch {
    /* ignore */
  }
}

function ageEligibleText(args: { rel: string }): string {
  if (args.rel === '—') return 'Older than minimum age';
  return args.rel;
}

function ageUnknownHours(args: { rel: string }): string {
  if (args.rel === '—') return 'Recent';
  return args.rel;
}

function ageRecentText(args: { rel: string; minAgeHours: number }): string {
  if (args.rel === '—') return `Recent (< ${args.minAgeHours}h)`;
  return `${args.rel} (< ${args.minAgeHours}h)`;
}

export function formatModifiedAge(args: { mtimeUnixSecs: string | null | undefined; ageEligible: boolean; minAgeHours?: number }): string {
  const rel = relativeAge({ secs: args.mtimeUnixSecs ?? null });
  if (args.ageEligible) return ageEligibleText({ rel });
  if (args.minAgeHours == null) return ageUnknownHours({ rel });
  return ageRecentText({ rel, minAgeHours: args.minAgeHours });
}
