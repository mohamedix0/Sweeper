import type { LogMeta } from './logParsers';
import { LogEntryCard } from './LogEntryCard';

export function LogsEntryList(props: {
  logs: LogMeta[];
  filtered: LogMeta[];
  contentById: Record<string, string>;
  selectedId: string | null;
  selectedContent: string | null;
  onOpen: (id: string) => void;
}) {
  if (props.logs.length === 0) return <p className="muted pad-md">No activity yet.</p>;
  if (props.filtered.length === 0) return <p className="muted pad-md">No matching activity.</p>;
  return (
    <ul className="log-list" aria-label="Log list">
      {props.filtered.map((l) => {
        const content = props.selectedId === l.id ? props.selectedContent : props.contentById[l.id] ?? null;
        return <LogEntryCard key={l.id} log={l} content={content} active={props.selectedId === l.id} onOpen={props.onOpen} />;
      })}
    </ul>
  );
}
