export type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
export type AuditRow = { type: string; category: string; location: string; size: string; outcome: string; outcomeTone: 'eligible' | 'cleaned' | 'skipped' | 'neutral'; kind: 'Category' | 'File'; sublabel?: string };
export type LogFilter = 'all' | 'scan' | 'clean';
export type OutcomeFilter = 'all' | 'eligible' | 'skipped';
export type Flags = { isClean: boolean; isScan: boolean };
export type LogSummary = { title: string; subtitle: string; when: string; whenAt: string; kind: LogFilter };
export type BaseSummary = Pick<LogSummary, 'when' | 'whenAt'>;
export type ActivitySummary = { primaryLabel: string; primaryValue: string; eligible: string; skipped: string; outcome: string; mode: string | null };
