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

  it('renders Activity History list and opens a selected log entry', async () => {
    const user = userEvent.setup();
    const { meta, content } = makeLog('log-1', 'one');
    const client = {
      listLogs: async () => [meta],
      readLog: async () => ({ id: 'log-1', content }),
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);
    expect(await screen.findByRole('heading', { name: 'Activity History' })).toBeInTheDocument();
    await screen.findByRole('button', { name: /System Scan/i });
    await user.click(screen.getByRole('button', { name: /System Scan/i }));
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
});
