import type { AuditRow } from './logParseTypes';

type Outcome = { label: string; tone: AuditRow['outcomeTone'] };
type Fallback = 'eligible' | 'cleaned' | 'skipped' | 'neutral';
function cleaned(): Outcome { return { label: 'Cleaned', tone: 'cleaned' }; }
function safe(): Outcome { return { label: 'Safe', tone: 'eligible' }; }
function skipped(): Outcome { return { label: 'Skipped', tone: 'skipped' }; }
function skippedUnverified(): Outcome { return { label: 'Skipped · Unverified', tone: 'skipped' }; }
function skippedAge(): Outcome { return { label: 'Skipped · < 24h', tone: 'skipped' }; }
function isUnknownSkip(args: { s: string }): boolean { if (args.s.includes('unknown')) return true; return args.s.includes('unverified'); }
function isAgeSkip(args: { s: string }): boolean { if (args.s.includes('< 24')) return true; return args.s.includes('24h'); }
export function skippedOutcomeLabel(args: { s: string }): Outcome {
  if (isUnknownSkip(args)) return skippedUnverified();
  if (isAgeSkip(args)) return skippedAge();
  return skipped();
}
function neutral(args: { raw: unknown }): Outcome { if (args.raw) return { label: String(args.raw), tone: 'neutral' }; return { label: 'Recorded', tone: 'neutral' }; }
export function fallbackOutcome(args: { fallback: Fallback; raw: unknown }): Outcome {
  if (args.fallback === 'cleaned') return cleaned();
  if (args.fallback === 'eligible') return safe();
  if (args.fallback === 'skipped') return skipped();
  return neutral(args);
}
export function normalizeOutcome(args: { raw: unknown; fallback: Fallback }): Outcome {
  const s = String(args.raw ?? '').toLowerCase();
  if (s.includes('clean')) return cleaned();
  if (s.includes('eligible')) return safe();
  if (s.includes('safe')) return safe();
  if (s.includes('skip')) return skippedOutcomeLabel({ s });
  return fallbackOutcome(args);
}


