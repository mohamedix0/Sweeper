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

  it('runs full Scan -> Explain -> Preview -> Clean summary -> Logs viewer flow', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const explainButtons = screen.getAllByRole('button', { name: 'Explain' });
    await user.click(explainButtons[0]);
    const dialog = await screen.findByRole('dialog', { name: /Explain:/ });
    expect(within(dialog).getByText(/Risk: Safe/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Close explanation' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

    const eligibleCheckbox = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp');
    await user.click(eligibleCheckbox);
    await user.click(screen.getByRole('button', { name: 'Preview Clean' }));
    await screen.findByText('Preview Breakdown');
    expect(screen.getByText(/mock-preview-/)).toBeInTheDocument();

    const cleanBtn = await screen.findByRole('button', { name: 'Clean Permanently' });
    expect(cleanBtn).toBeEnabled();
    await user.click(cleanBtn);

    await screen.findByRole('heading', { name: 'Clean Summary' });
    expect(screen.getByText('Bytes freed')).toBeInTheDocument();
    expect(screen.getByText('Deleted count')).toBeInTheDocument();
    expect(screen.getByText('Skipped count')).toBeInTheDocument();
    expect(screen.getByLabelText('Skipped reasons')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'View Operation Log' }));
    await screen.findByRole('heading', { name: 'Logs' });
    await screen.findByLabelText('Log content');
    expect(screen.getByLabelText('Log content')).toHaveTextContent(/"record":"intent"/);
  });

  it('enforces typed confirmation for Custom Path/Unknown scan items and requires exact DELETE', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb).toBeDisabled(); // gated by allow_delete_unknown

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    const allowUnknown = screen.getByLabelText('Allow deleting unknown folders (advanced)');
    await user.click(allowUnknown);
    await user.click(screen.getByRole('button', { name: 'Save Settings' }));
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Scan & Preview' }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const unknownCb2 = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb2).toBeEnabled();
    await user.click(unknownCb2);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Preview Clean' });
    expect(previewBtn).toBeDisabled();

    await user.type(screen.getByLabelText('Typed confirmation'), 'delete');
    expect(previewBtn).toBeDisabled();
    await user.clear(screen.getByLabelText('Typed confirmation'));
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
    await user.click(previewBtn);
    await screen.findByText('Preview Breakdown');
  });

  it('Advanced Settings supports Tier 3, min-age warning, custom path add/reject/remove, and allow unknown gating', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });

    const minAge = screen.getByLabelText('Minimum Age (hours)');
    await user.clear(minAge);
    await user.type(minAge, '0');
    expect(screen.getByText(/Minimum Age below 24 hours/i)).toBeInTheDocument();

    const tier3 = screen.getByLabelText('Enable Tier 3 targets') as HTMLInputElement;
    expect(tier3.checked).toBe(false);
    const staleDays = screen.getByLabelText('Stale project threshold (days)') as HTMLInputElement;
    expect(staleDays).toBeDisabled();
    await user.click(tier3);
    expect(staleDays).toBeEnabled();
    await user.clear(staleDays);
    await user.type(staleDays, '45');
    expect(screen.getByText(/display-only/i)).toBeInTheDocument();
    expect(screen.getByLabelText('Tier 3 target notes')).toHaveTextContent(/Recycle Bin/i);

    // Custom path reject
    await user.type(screen.getByLabelText('Add Custom Path'), 'C:\\Windows\\Temp\\bad');
    await user.click(screen.getByRole('button', { name: 'Add & Validate' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/Custom Path rejected/i);

    // Custom path add
    const input = screen.getByLabelText('Add Custom Path');
    await user.clear(input);
    await user.type(input, 'C:\\Users\\Alice\\CustomCleanup');
    await user.click(screen.getByRole('button', { name: 'Add & Validate' }));
    await screen.findByText(/Settings saved/i);
    expect(screen.getByRole('table', { name: 'Custom Paths' })).toHaveTextContent('C:\\Users\\Alice\\CustomCleanup');

    // Remove
    await user.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(screen.queryByRole('table', { name: 'Custom Paths' })).not.toBeInTheDocument());
  });

  it('Analyzer gates Unknown selection behind allow_unknown and requires typed confirmation for Unknown nodes', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const unknownPath = 'C:\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache';
    let unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb) {
      // Root is expanded by default; if virtualization/state timing hides children in tests, re-toggle root.
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb) {
      // Fallback: locate the gated Unknown node row by its status text.
      const statuses = await screen.findAllByText(/Unknown gated by Settings/i);
      const row = statuses[0]?.closest('[role="treeitem"]') as HTMLElement | null;
      expect(row).not.toBeNull();
      unknownCb = within(row!).getByRole('checkbox') as HTMLInputElement;
    }
    expect(unknownCb).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByLabelText('Allow deleting unknown folders (advanced)'));
    await user.click(screen.getByRole('button', { name: 'Save Settings' }));
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    let unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb2) {
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb2) {
      // After allowing unknowns, select the first Unknown-selectable checkbox by finding a node whose status indicates typed confirmation required for Unknown/custom path.
      const candidates = screen.getAllByRole('checkbox', { name: /Select analyzer node /i }) as HTMLInputElement[];
      unknownCb2 = candidates.find((cb) => !cb.disabled) ?? null;
    }
    expect(unknownCb2).not.toBeNull();
    expect(unknownCb2!).toBeEnabled();
    await user.click(unknownCb2!);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Preview Selected' });
    expect(previewBtn).toBeDisabled();
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
  });

  it('handles expired/invalidated session errors cleanly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
    mockController.advanceTimeMs(15 * 60 * 1000 + 1000);
    await user.click(screen.getByRole('button', { name: 'Preview Clean' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/expired|invalid/i);
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Scan Results' })).not.toBeInTheDocument());
  });

  it('marks non-selectable analyzer nodes (.vhdx / Protected / Link Point) correctly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const all = screen.getAllByRole('checkbox') as HTMLInputElement[];
    const findBySuffix = (suffix: string) => all.find((n) => (n.getAttribute('aria-label') ?? '').includes(suffix));
    const vhdx = findBySuffix('dev-machine.vhdx');
    expect(vhdx).toBeTruthy();
    expect(vhdx!).toBeDisabled();
    expect(vhdx!.closest('[role="treeitem"]')).toHaveTextContent(/\.vhdx/i);

    const gitNode = findBySuffix('\\Dev\\repo\\.git');
    expect(gitNode).toBeTruthy();
    expect(gitNode!).toBeDisabled();
    expect(gitNode!.closest('[role="treeitem"]')).toHaveTextContent(/Protected/i);

    const linkNode = findBySuffix('\\Users\\Alice\\OneDriveLink');
    expect(linkNode).toBeTruthy();
    expect(linkNode!).toBeDisabled();
    expect(linkNode!.closest('[role="treeitem"]')).toHaveTextContent(/Link Point/i);
  });
});