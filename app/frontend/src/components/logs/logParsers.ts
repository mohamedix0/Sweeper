export type { AuditRow, LogFilter, LogMeta, OutcomeFilter } from './logParseTypes';
export { formatClockTime, formatLogWhen } from './logParseUtils';
export { normalizeOutcome } from './logParseOutcome';
export { summarizeLog } from './logParseSummary';
export { deriveActivitySummary } from './logParseActivity';
import type { AuditRow } from './logParseTypes';
import { parseAuditRows as parseAuditRowsInternal } from './logParseRows';

export function parseAuditRows(args: { content: string; meta: unknown }): AuditRow[] {
  void args.meta;
  return parseAuditRowsInternal(args.content);
}


