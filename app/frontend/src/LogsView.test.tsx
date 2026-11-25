import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { LogsView } from './components/LogsView';
import type { SweeperClient } from './api/client';

function makeLog(id: string, label: string) {
  const content = JSON.stringify({ record: 'intent', action: 'scan', label }) + '\n' + JSON.stringify({ record: 'outcome', event: 'scan', categoriesCount: 1, totalBytes: '1024' });
  return { meta: { id, name: `${id}.jsonl`, sizeBytes: String(content.length), modifiedUnixSecs: '1710000000' }, content };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
  });

  it('does not advertise an unused Activity-log refresh capability (L2, #148)', () => {
    // useLogData used to build and return a `refresh` callback that no caller
    // ever invoked. Keep it removed so the code stops advertising a capability
    // it does not wire up. The real refresh worker must remain: the mount and
    // focus paths rely on it to load the Activity list (#148 narrow scope only).
    const source = readFileSync(resolve(__dirname, 'components/logs/useLogsSelectionModel.tsx'), 'utf8');
    expect(source).not.toMatch(/\bconst\s+refresh\s*=/);
    expect(source).not.toMatch(/\brefresh\s*,/);
    expect(source).not.toMatch(/cancelled:\s*\(\)\s*=>\s*false/);
    expect(source).toMatch(/refreshLogs\(/);
  });

  it('renders Activity History list and opens a selected log entry', async () => {
    const user = userEvent.setup();
    const { meta, content } = makeLog('log-1', 'one');
    const client = {
      listLogs: async () => [meta],
      readLog: async () => ({ id: 'log-1', content }),
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);
    expect(await screen.findByRole('heading', { name: 'Activity History' })).toBeInTheDocument();
    await screen.findByRole('button', { name: /Scan Completed/i });
    await user.click(screen.getByRole('button', { name: /Scan Completed/i }));
    await screen.findByLabelText('Structured log summary');
    await user.click(screen.getByRole('button', { name: 'Raw JSONL' }));
    expect(await screen.findByLabelText('Log content')).toHaveTextContent(/"record":"intent"/);
  });

  it('ignores stale log reads when selection changes quickly', async () => {
    const user = userEvent.setup();
    const a = makeLog('log-a', 'alpha');
    const b = makeLog('log-b', 'beta');
    const first = deferred<{ id: string; content: string }>();
    const client = {
      listLogs: async () => [a.meta, b.meta],
      readLog: async (id: string) => {
        if (id === 'log-a') return first.promise;
        if (id === 'log-b') return { id, content: b.content };
        return { id, content: a.content };
      },
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);
    const list = await screen.findByLabelText('Log list');
    const buttons = await within(list).findAllByRole('button');
    await user.click(buttons[0]!);
    await user.click(buttons[1]!);
    await screen.findByLabelText('Structured log summary');
    await user.click(screen.getByRole('button', { name: 'Raw JSONL' }));
    await waitFor(() => expect(screen.getByLabelText('Log content')).toHaveTextContent(/"label":"beta"/));
    first.resolve({ id: 'log-a', content: a.content });
    await waitFor(() => expect(screen.getByLabelText('Log content')).toHaveTextContent(/"label":"beta"/));
    expect(screen.getByLabelText('Log content')).not.toHaveTextContent(/"label":"alpha"/);
  });

  it('renders journal clean outcomes with reclaimed bytes and counts', async () => {
    const content = [
      JSON.stringify({ record: 'intent', session_id: 'session-1' }),
      JSON.stringify({ record: 'outcome', target_id: 'temp-a', path: 'C:\\Temp\\a.tmp', bytes: 1024, status: 'deleted' }),
      JSON.stringify({ record: 'outcome', target_id: 'temp-b', path: 'C:\\Temp\\b.tmp', bytes: 2048, status: 'deleted' }),
      JSON.stringify({ record: 'outcome', target_id: 'temp-a', path: 'C:\\Temp\\c.tmp', bytes: 512, status: 'skipped' }),
    ].join('\n');
    const meta = { id: 'clean-1', name: 'clean-1.jsonl', sizeBytes: String(content.length), modifiedUnixSecs: '1710000000' };
    const client = {
      listLogs: async () => [meta],
      readLog: async () => ({ id: 'clean-1', content }),
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);

    const summary = await screen.findByLabelText('Activity summary');
    expect(summary).toHaveTextContent('3 KB');
    expect(summary).toHaveTextContent('2 files');
    expect(summary).toHaveTextContent('2 categories');
    expect(summary).toHaveTextContent('1 file');
    expect(summary).toHaveTextContent('1 category');
    expect(summary).not.toHaveTextContent(/^0 B$/);
    expect(summary).not.toHaveTextContent('0 files · 0 categories');
  });

  it('clears stale cleanup details when switching to an empty Scans filter', async () => {
    const user = userEvent.setup();
    const content = [
      JSON.stringify({ record: 'intent', session_id: 'session-2' }),
      JSON.stringify({ record: 'outcome', target_id: 'temp-a', path: 'C:\\Temp\\a.tmp', bytes: 1024, status: 'deleted' }),
    ].join('\n');
    const meta = { id: 'clean-2', name: 'clean-2.jsonl', sizeBytes: String(content.length), modifiedUnixSecs: '1710000001' };
    let listCalls = 0;
    let readCalls = 0;
    const client = {
      listLogs: async () => {
        listCalls += 1;
        return [meta];
      },
      readLog: async () => {
        readCalls += 1;
        return { id: 'clean-2', content };
      },
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);

    await screen.findByLabelText('Structured log summary');
    await waitFor(() => expect(listCalls).toBeGreaterThan(0));
    await waitFor(() => expect(readCalls).toBeGreaterThan(0));
    await user.click(screen.getByRole('button', { name: 'Scans' }));

    await screen.findByText('No matching activity for this filter');
    expect(screen.queryByLabelText('Structured log summary')).not.toBeInTheDocument();

    const settledList = listCalls;
    const settledRead = readCalls;
    await new Promise((r) => setTimeout(r, 120));
    expect(listCalls).toBe(settledList);
    expect(readCalls).toBe(settledRead);
  });
});
