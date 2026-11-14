import { useCallback, useEffect, useState } from 'react';
import { formatContractError, type SweeperClient } from '../api/client';
import { formatBytes, formatUnixSeconds } from '../utils/format';

export function LogsView({ client, focusLogId, onFocusConsumed }: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void }) {
  const [logs, setLogs] = useState<{ id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null }[]>([]);
  const [selected, setSelected] = useState<{ id: string; content: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await client.listLogs();
      setLogs(next);
    } catch (e) {
      setError(formatContractError(e));
    }
  }, [client]);

  useEffect(() => { void refresh(); }, [refresh]);

  useEffect(() => {
    if (!focusLogId) return;
    let cancelled = false;
    (async () => {
      setError(null);
      try {
        await refresh();
        const log = await client.readLog(focusLogId);
        if (!cancelled) setSelected(log);
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) onFocusConsumed?.();
      }
    })();
    return () => { cancelled = true; };
  }, [focusLogId, client, refresh, onFocusConsumed]);

  async function openLog(id: string) {
    setError(null);
    try { setSelected(await client.readLog(id)); } catch (e) { setError(formatContractError(e)); }
  }

  return (
    <section className="panel" aria-labelledby="logs-heading">
      <h2 id="logs-heading">Logs</h2>
      <p className="muted">Operation log inspector (<code>list_logs</code> / <code>read_log</code>).</p>
      {error && <div className="notice error" role="alert">{error}</div>}
      <div className="grid-2">
        <div>
          <div className="row">
            <h3 className="section-title">Available Logs</h3>
            <div className="spacer" />
            <button className="btn" onClick={() => refresh()}>Refresh</button>
          </div>
          {logs.length === 0 ? <p className="muted">No logs found.</p> : (
            <table className="table" aria-label="Log list">
              <thead><tr><th>Name</th><th>Modified</th><th>Size</th><th></th></tr></thead>
              <tbody>
                {logs.map((l) => (
                  <tr key={l.id}>
                    <td>{l.name}</td>
                    <td>{formatUnixSeconds(l.modifiedUnixSecs)}</td>
                    <td>{l.sizeBytes ? formatBytes(l.sizeBytes) : '—'}</td>
                    <td><button className="btn" onClick={() => openLog(l.id)}>View</button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        <div>
          <h3 className="section-title">Log Content</h3>
          {selected ? (
            <div>
              <p className="muted">Log: {selected.id}</p>
              <pre className="log" aria-label="Log content">{selected.content}</pre>
            </div>
          ) : <p className="muted">Select a log to inspect its contents.</p>}
        </div>
      </div>
    </section>
  );
}
