import type { LogMeta } from './logParsers';
import { summarizeLog, type LogFilter } from './logParsers';
import { CheckIcon } from '../Icons';

export function LogEntryCard(props: {
  log: LogMeta;
  content: string | null;
  active: boolean;
  onOpen: (id: string) => void;
}) {
  const sum = summarizeLog(props.log, props.content);
  return (
    <li className="log-list-item">
      <button type="button" className={`log-select-btn ${props.active ? 'active' : ''}`} onClick={() => props.onOpen(props.log.id)}>
        <span className="log-primary">{sum.title}</span>
        <span className="log-secondary"><span>{sum.when}</span><span className="log-status-check" aria-label={sum.subtitle}><CheckIcon /> <span>{sum.subtitle}</span></span></span>
      </button>
    </li>
  );
}

export function matchesLogFilter(args: { meta: LogMeta; content: string | null; filter: LogFilter; query: string }): boolean {
  const sum = summarizeLog(args.meta, args.content);
  if (args.filter !== 'all' && sum.kind !== args.filter) return false;
  if (!args.query) return true;
  const content = args.content ?? '';
  const hay = [sum.title, sum.when, content, args.meta.name].join('\n').toLowerCase();
  return hay.includes(args.query);
}
