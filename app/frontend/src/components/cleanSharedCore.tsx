import { normalizeDisplayPath, pluralize, relativeAge } from '../utils/format';

export type ExplainState = { itemId: string; title: string; description: string; risk: import('../api/client').RiskDto; failed?: boolean } | null;

/** Shared fallback copy when the Knowledge Base explanation cannot be shown. */
export const EXPLANATION_UNAVAILABLE = 'Explanation unavailable for this item — showing its known identity and risk only.';

/** Known identity/Risk of an item, used to build a fallback explanation on IPC failure. */
export type ExplanationFallback = { title?: string; risk?: import('../api/client').RiskDto };

/**
 * Synthesize an ExplainState for a failed `explain_item` call. Identity/Risk
 * come from the already-known Scan result or Analyzer node (the inspector also
 * renders them from that context), and `failed` drives the explicit notice.
 */
export function failedExplanation(args: { itemId: string; known?: ExplanationFallback }): NonNullable<ExplainState> {
  return {
    itemId: args.itemId,
    title: args.known?.title ?? args.itemId,
    description: EXPLANATION_UNAVAILABLE,
    risk: args.known?.risk ?? 'Unknown',
    failed: true,
  };
}

/** Explicit notice shown in the inspector when an explanation could not be loaded. */
export function ExplanationUnavailableNotice() {
  return <div className="notice warn" role="note" data-testid="explanation-unavailable-notice">{EXPLANATION_UNAVAILABLE}</div>;
}

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
