import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { InfoTip } from './components/cleanShared';
import { mockController } from './api/mock';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  function expectScanShellReady() {
    expect(screen.getAllByText('Sweeper').length).toBeGreaterThan(0);
    const nav = screen.getByRole('button', { name: 'Scan & Clean' });
    expect(nav).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('heading', { name: 'Scan & Clean' })).toBeInTheDocument();
    const start = screen.getByRole('button', { name: 'Start Scan' });
    expect(start).toBeEnabled();
  }

  it('renders Scan & Clean shell and primary navigation', async () => {
    setup();
    expectScanShellReady();
  });

  it('shows locked system-wide cleanup notice on Scan & Clean when non-elevated', async () => {
    setup();
    expect((await screen.findAllByLabelText('Standard (Non-Elevated)')).length).toBeGreaterThan(0);
    expect(screen.getAllByRole('button', { name: /Standard Mode/i }).length).toBeGreaterThan(0);
  });

  it('toggles Dark/Light theme and persists choice in localStorage', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    const appearance = screen.getByRole('group', { name: 'Appearance' });
    await user.click(within(appearance).getByRole('button', { name: 'Light' }));
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(window.localStorage.getItem('sweeper-theme')).toBe('light');
    await user.click(within(appearance).getByRole('button', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(window.localStorage.getItem('sweeper-theme')).toBe('dark');
  });

  it('InfoTip pins on first click, closes on second click, and exposes matching aria-describedby without dialog popup semantics', async () => {
    const user = userEvent.setup();
    render(
      <InfoTip label="About example">
        <p>Tooltip details</p>
      </InfoTip>,
    );
    const infoBtn = screen.getByRole('button', { name: 'About example' });
    expect(infoBtn).not.toHaveAttribute('aria-haspopup', 'dialog');

    await user.click(infoBtn);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Tooltip details');
    expect(infoBtn).toHaveAttribute('aria-describedby', tooltip.id);
    expect(infoBtn).toHaveAttribute('aria-expanded', 'true');

    await user.click(infoBtn);
    await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument());
    expect(infoBtn).not.toHaveAttribute('aria-describedby');

    await user.click(infoBtn);
    await screen.findByRole('tooltip');
    infoBtn.focus();
    fireEvent.keyDown(infoBtn, { key: 'Escape', code: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument());
  });

  it('runs full Scan -> Inspect -> Preview -> Clean summary -> Logs viewer flow', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    // auto-inspected item renders contextual explanation in Inspector (no Explain button/drawer)
    await waitFor(async () => {
      expect(await screen.findByText(/Temporary files Windows and apps can recreate automatically/i)).toBeInTheDocument();
    });

    const eligibleCheckbox = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    if (!eligibleCheckbox.checked) await user.click(eligibleCheckbox);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await screen.findByText('Preview Breakdown');

    const cleanBtn = await screen.findByRole('button', { name: /Delete .* permanently/ });
    expect(cleanBtn).toBeEnabled();
    await user.click(cleanBtn);

    await screen.findByRole('heading', { name: 'Clean Summary' });
    function expectCleanSummaryFields() {
      expect(screen.getByText('Bytes freed')).toBeInTheDocument();
      const deleted = screen.getByText('Deleted count');
      expect(deleted).toBeInTheDocument();
      expect(screen.getByText('Skipped count')).toBeInTheDocument();
      const reasons = screen.getByLabelText('Skipped reasons');
      expect(reasons).toBeInTheDocument();
    }
    expectCleanSummaryFields();

    await user.click(screen.getByRole('button', { name: 'View Operation Log' }));
    await screen.findByRole('heading', { name: 'Activity History' });
    expect(screen.getByRole('button', { name: 'Summary' })).toHaveAttribute('aria-pressed', 'true');
    await screen.findByLabelText('Structured log summary');
    await user.click(screen.getByRole('button', { name: 'Raw JSONL' }));
    await screen.findByLabelText('Log content');
    expect(screen.getByLabelText('Log content')).toHaveTextContent(/"record":"intent"/);
  });

  it('shows non-elevated status, locks elevation targets, and reports UAC cancellation feedback safely', async () => {
    mockController.setRelaunchResult({
      error: { code: 'internal', details: { reason: 'Administrator launch was cancelled.' } },
    });
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    expect(screen.getAllByLabelText('Standard (Non-Elevated)').length).toBeGreaterThan(0);

    const elevationToggle = screen.getByLabelText(
      'Allow system-wide Windows caches (Admin)',
    ) as HTMLInputElement;
    expect(elevationToggle).toBeDisabled();
    expect(screen.getByText(/Deep Cleanup must be enabled first/i)).toBeInTheDocument();

    await user.click(screen.getAllByRole('button', { name: 'Run as administrator' })[0]);
    expect((await screen.findAllByRole('alert')).some((el) => /Administrator launch was cancelled/i.test(el.textContent || ''))).toBe(true);
  });

  it('allows enabling and persisting elevation targets when elevated', async () => {
    mockController.setElevationStatus({ isElevated: true });
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    expect(screen.getAllByLabelText('Administrator (Elevated)').length).toBeGreaterThan(0);

    await user.click(screen.getByLabelText('Enable deep cleanup categories'));
    const elevationToggle = screen.getByLabelText(
      'Allow system-wide Windows caches (Admin)',
    ) as HTMLInputElement;
    expect(elevationToggle).toBeEnabled();
    await user.click(elevationToggle);
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    const saved = await mockController.getSettings();
    expect(saved.enableElevationTargets).toBe(true);
  });

  it('preserves Scan & Clean and Analyzer state across navigation (Bug 4)', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    expect(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
    expect(await screen.findByRole('heading', { name: 'Inspector' })).toBeInTheDocument();
    expect(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    expect(await screen.findByRole('heading', { name: 'Analyzer Tree' })).toBeInTheDocument();
  });
});
