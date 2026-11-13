import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    mockController.reset();
  });

  it('renders Scan & Preview shell and primary navigation', async () => {
    setup();
    expect(screen.getByText('Sweeper')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Scan & Preview' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('heading', { name: 'Scan & Preview' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
  });

  it('runs Scan -> Explain -> Preview workflow with BigInt totals', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));

    await screen.findByRole('heading', { name: 'Scan Results' });
    expect(screen.getByText('User Temp')).toBeInTheDocument();
    expect(screen.getByText(/Selected: 0 \(0 B\)/)).toBeInTheDocument();

    // Explain first eligible item
    const explainButtons = screen.getAllByRole('button', { name: 'Explain' });
    await user.click(explainButtons[0]);
    const dialog = await screen.findByRole('dialog', { name: /Explain:/ });
    expect(within(dialog).getByText(/Risk: Safe/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Close explanation' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

    // Select eligible items (checkboxes for eligible paths)
    const eligibleCheckbox = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp');
    await user.click(eligibleCheckbox);
    expect(screen.getByText(/Selected: 1 \(1 MB\)/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Preview Clean (read-only)' }));
    await screen.findByText('Preview Breakdown');
    expect(screen.getByText(/mock-preview-/)).toBeInTheDocument();
    expect(screen.getByText(/Total selected:/).closest('p')).toHaveTextContent('1 item(s), 1 MB');
    expect(screen.getByRole('table', { name: 'Preview breakdown by Target' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Clean (Phase 3 — locked)' })).toBeDisabled();
  });

  it('enforces typed confirmation when required and recovers from invalid input', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    // Select Unknown eligible item -> typed confirmation required
    const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache\\old.bin');
    await user.click(unknownCb);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Preview Clean (read-only)' });
    expect(previewBtn).toBeDisabled();

    await user.type(screen.getByLabelText('Typed confirmation'), 'WRONG');
    expect(previewBtn).toBeEnabled();
    await user.click(previewBtn);
    expect(await screen.findByRole('alert')).toHaveTextContent(/Typed confirmation required/i);

    // recover with correct confirmation
    const input = screen.getByLabelText('Typed confirmation');
    await user.clear(input);
    await user.type(input, 'DELETE');
    await user.click(screen.getByRole('button', { name: 'Preview Clean (read-only)' }));
    await screen.findByText('Preview Breakdown');
    expect(screen.getByText(/Total selected:/).closest('p')).toHaveTextContent('8 KB');
  });

  it('hides typed confirmation when not required', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
    expect(screen.queryByLabelText('Typed confirmation')).not.toBeInTheDocument();
  });

  it('shows structured ContractError for expired session and resets state', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));

    // expire session (15m)
    mockController.advanceTimeMs(15 * 60 * 1000 + 1000);
    await user.click(screen.getByRole('button', { name: 'Preview Clean (read-only)' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/expired|invalid/i);
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Scan Results' })).not.toBeInTheDocument());
  });

  it('shows structured ContractError for invalidated session (settings generation)', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));

    mockController.invalidateSettings();
    await user.click(screen.getByRole('button', { name: 'Preview Clean (read-only)' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/invalidated|settings/i);
  });

  it('renders Settings (read-only) view', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings (Read-Only)' }));
    expect(await screen.findByRole('heading', { name: 'Settings (Read-Only in Phase 2)' })).toBeInTheDocument();
    expect(screen.getByText('Tier 1 enabled')).toBeInTheDocument();
    expect(screen.getByText('Minimum Age (hours)')).toBeInTheDocument();
    expect(screen.getByText('Allow deleting unknown folders')).toBeInTheDocument();
    expect(screen.getByText(/Not editable in Phase 2/)).toBeInTheDocument();
  });

  it('renders Logs view and reads a log', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Logs' }));
    expect(await screen.findByRole('heading', { name: 'Logs' })).toBeInTheDocument();
    expect(await screen.findByRole('table', { name: 'Log list' })).toBeInTheDocument();
    const viewButtons = await screen.findAllByRole('button', { name: 'View' });
    await user.click(viewButtons[0]);
    expect(await screen.findByLabelText('Log content')).toHaveTextContent(/"event":"scan"/);
  });

  it('does not allow selecting Protected or ineligible items', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const protectedCb = screen.getByLabelText('Select C:\\Windows\\System32\\kernel32.dll') as HTMLInputElement;
    expect(protectedCb).toBeDisabled();

    const recentCb = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\recent.log') as HTMLInputElement;
    expect(recentCb).toBeDisabled();

    // selecting eligible only updates total accordingly
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
    expect(screen.getByText(/Selected: 1 \(/)).toBeInTheDocument();
  });

  it('has basic accessibility labels/roles for key controls', async () => {
    setup();
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument();
    expect(screen.getByLabelText('Tier 1 (system temp and caches)')).toBeInTheDocument();
    expect(screen.getByLabelText('Tier 2 (developer caches)')).toBeInTheDocument();
  });
});
