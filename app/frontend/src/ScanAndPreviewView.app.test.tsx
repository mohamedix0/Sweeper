import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

async function startScan(input: { user: ReturnType<typeof userEvent.setup> }) {
  await input.user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
}

async function runTypedConfirmCase() {
  const { user } = setup();
  await startScan({ user });
  const expandBtn = screen.queryByRole('button', { name: /Expand Custom Path/i });
  if (expandBtn) await user.click(expandBtn);
  const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
  expect(unknownCb).toBeDisabled();
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  await screen.findByRole('heading', { name: 'Settings' });
  await user.click(screen.getByLabelText('Allow deleting unknown folders (advanced)'));
  await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
  await screen.findByText(/Settings saved/i);
  await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
  await startScan({ user });
  const expandBtn2 = screen.queryByRole('button', { name: /Expand Custom Path/i });
  if (expandBtn2) await user.click(expandBtn2);
  const unknownCb2 = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
  expect(unknownCb2).toBeEnabled();
  await user.click(unknownCb2);
  expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
  const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
  expect(previewBtn).toBeDisabled();
  await user.type(screen.getByLabelText('Typed confirmation'), 'delete');
  expect(previewBtn).toBeDisabled();
  await user.clear(screen.getByLabelText('Typed confirmation'));
  await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
  expect(previewBtn).toBeEnabled();
  await user.click(previewBtn);
  await screen.findByText('Preview Breakdown');
}

async function runExpiredCase() {
  const { user } = setup();
  await startScan({ user });
  await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
  mockController.advanceTimeMs(15 * 60 * 1000 + 1000);
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  const alert = await screen.findByRole('alert');
  expect(alert).toHaveTextContent(/expired|invalid/i);
  await waitFor(() => expect(screen.queryByRole('heading', { name: 'Inspector' })).not.toBeInTheDocument());
}

async function runBulkSelectionCase() {
  const { user } = setup();
  await startScan({ user });
  await user.click(screen.getByRole('button', { name: 'Select Safe' }));
  const safeEligible = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
  expect(safeEligible).toBeChecked();
  expect(screen.getAllByLabelText('Status Skipped').length).toBeGreaterThan(0);
  await user.click(screen.getByRole('button', { name: /^Eligible \(/ }));
  await waitFor(() => expect(screen.queryAllByLabelText('Status Skipped')).toHaveLength(0));
  await user.click(screen.getAllByRole('button', { name: 'Clear' })[0]);
  await waitFor(() => expect(safeEligible.checked).toBe(false));
  await user.click(screen.getByRole('button', { name: /^All \(/ }));
  const userTempGroup = screen.getByLabelText('Select all eligible items in User Temp') as HTMLInputElement;
  await user.click(userTempGroup);
  expect(safeEligible).toBeChecked();
}

describe('ScanAndPreviewView app flows', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('enforces typed confirmation for Custom Path/Unknown scan items and requires exact DELETE', async () => { await runTypedConfirmCase(); });
  it('handles expired/invalidated session errors cleanly', async () => { await runExpiredCase(); });
  it('supports scan bulk selection, target-group selection, and status filters', async () => { await runBulkSelectionCase(); });
});