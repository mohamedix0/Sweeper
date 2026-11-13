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

  it('enforces typed confirmation when required and recovers from invalid input', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache\\old.bin');
    await user.click(unknownCb);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Preview Clean' });
    expect(previewBtn).toBeDisabled();

    await user.type(screen.getByLabelText('Typed confirmation'), 'WRONG');
    expect(previewBtn).toBeEnabled();
    await user.click(previewBtn);
    expect(await screen.findByRole('alert')).toHaveTextContent(/Typed confirmation required/i);

    const input = screen.getByLabelText('Typed confirmation');
    await user.clear(input);
    await user.type(input, 'DELETE');
    await user.click(screen.getByRole('button', { name: 'Preview Clean' }));
    await screen.findByText('Preview Breakdown');
    expect(screen.getByText(/Total selected:/).closest('p')).toHaveTextContent('8 KB');
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

  it('handles invalidated session during preview (settings generation)', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));

    mockController.invalidateSettings();
    await user.click(screen.getByRole('button', { name: 'Preview Clean' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/invalidated|settings|expired/i);
  });

  it('renders Settings view', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('heading', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByText('Tier 1 enabled')).toBeInTheDocument();
    expect(screen.getByText('Minimum Age (hours)')).toBeInTheDocument();
    expect(screen.getByText('Allow deleting unknown folders')).toBeInTheDocument();
  });

  it('renders Logs view and reads a log', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Logs' }));
    expect(await screen.findByRole('heading', { name: 'Logs' })).toBeInTheDocument();
    expect(await screen.findByRole('table', { name: 'Log list' })).toBeInTheDocument();
    const viewButtons = await screen.findAllByRole('button', { name: 'View' });
    await user.click(viewButtons[0]);
    expect(await screen.findByLabelText('Log content')).toBeInTheDocument();
  });

  it('does not allow selecting Protected or ineligible items', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Scan Results' });

    const protectedCb = screen.getByLabelText('Select C:\\Windows\\System32\\kernel32.dll') as HTMLInputElement;
    expect(protectedCb).toBeDisabled();
    const recentCb = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\recent.log') as HTMLInputElement;
    expect(recentCb).toBeDisabled();
  });

  it('has basic accessibility labels/roles for key controls', async () => {
    setup();
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument();
    expect(screen.getByLabelText('Tier 1 (system temp and caches)')).toBeInTheDocument();
    expect(screen.getByLabelText('Tier 2 (developer caches)')).toBeInTheDocument();
  });

  it('runs Analyzer -> virtualized tree -> explain/select eligible -> Preview Selected -> Clean summary', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    expect(await screen.findByRole('heading', { name: 'Analyzer' })).toBeInTheDocument();
    const rootInput = screen.getByLabelText('Analyzer root path') as HTMLInputElement;
    expect(rootInput.value).toBe('C:\\');
    await user.click(screen.getByRole('button', { name: 'Analyze' }));

    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    const tree = screen.getByRole('tree', { name: 'Analyzer results tree' });
    expect(tree).toBeInTheDocument();

    // Virtualization: mock tree has many bulk nodes but viewport should not render all rows at once.
    const renderedRows = tree.querySelectorAll('[role="treeitem"]');
    expect(renderedRows.length).toBeGreaterThan(0);
    expect(renderedRows.length).toBeLessThan(60);

    // Expand Temp to expose a selectable eligible file.
    await user.click(screen.getByRole('button', { name: 'Expand Temp' }));
    const selectableNode = await waitFor(() => {
      const n = screen.getAllByRole('checkbox').find((el) => (el.getAttribute('aria-label') ?? '').includes('\\Temp\\a.tmp')) as HTMLInputElement | undefined;
      expect(n).toBeTruthy();
      return n!;
    });
    expect(selectableNode).toBeEnabled();
    await user.click(selectableNode);

    await user.click(screen.getAllByRole('button', { name: 'Explain' })[0]);
    expect(await screen.findByRole('dialog', { name: /Explain:/ })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close explanation' }));
    await user.click(screen.getByRole('button', { name: 'Preview Selected' }));
    await screen.findByText('Preview Breakdown');
    expect(screen.getByText(/Permanent deletion \(bypasses Recycle Bin\)/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Clean Permanently' }));
    await screen.findByRole('heading', { name: 'Clean Summary' });
    expect(screen.getByText('Bytes freed')).toBeInTheDocument();
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
    expect(vhdx!.closest('[role="treeitem"]')).toHaveTextContent(/Not selectable/i);
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
