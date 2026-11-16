import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LogsView } from './components/LogsView';
import type { SweeperClient } from './api/client';

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
  });

  it('renders Activity History list and opens a selected log entry', async () => {
    const user = userEvent.setup();
    const logContent = JSON.stringify({ record: 'intent', action: 'scan' }) + '\n' + JSON.stringify({ record: 'outcome', event: 'scan', categoriesCount: 1, totalBytes: '1024' });
    const client = {
      listLogs: async () => [{ id: 'log-1', name: 'scan-1.jsonl', sizeBytes: String(logContent.length), modifiedUnixSecs: '1710000000' }],
      readLog: async () => ({ id: 'log-1', content: logContent }),
    } as unknown as SweeperClient;

    render(<LogsView client={client} />);
    expect(await screen.findByRole('heading', { name: 'Activity History' })).toBeInTheDocument();
    await screen.findByRole('button', { name: /System Scan/i });
    await user.click(screen.getByRole('button', { name: /System Scan/i }));
    await screen.findByLabelText('Structured log summary');
    await user.click(screen.getByRole('button', { name: 'Raw JSONL' }));
    expect(await screen.findByLabelText('Log content')).toHaveTextContent(/"record":"intent"/);
  });
});
