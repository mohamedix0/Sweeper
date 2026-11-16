import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { SettingsView } from './components/SettingsView';
import { mockController } from './api/mock';
import type { Settings } from './api/client';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

function baseSettings(): Settings {
  return { enableTier1: true, enableTier2: true, enableTier3: false, enableElevationTargets: false, minAgeHours: 24, allowDeleteUnknown: false, customPaths: [], staleProjectDays: 30 };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('Advanced Settings supports Tier 3, min-age warning, custom path add/reject/remove, and allow unknown gating', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });

    const minAge = screen.getByLabelText('Minimum file age');
    await user.clear(minAge);
    await user.type(minAge, '0');
    expect(screen.getByText(/below 24 hours/i)).toBeInTheDocument();

    const tier3 = screen.getByLabelText('Enable deep cleanup categories') as HTMLInputElement;
    expect(tier3.checked).toBe(false);
    const staleDays = screen.getByLabelText('Inactive project cache age') as HTMLInputElement;
    expect(staleDays).toBeDisabled();
    await user.click(tier3);
    expect(staleDays).toBeEnabled();
    await user.clear(staleDays);
    await user.type(staleDays, '45');
    expect(screen.getByText(/Highlight inactive Rust build caches/i)).toBeInTheDocument();
    expect(screen.getByText(/Empty Windows Recycle Bin/i)).toBeInTheDocument();
    expect(screen.getByLabelText('Recycle Bin included with Deep Cleanup')).toHaveTextContent(/Included with Deep Cleanup/i);

    // Custom path reject
    await user.type(screen.getByLabelText('Add Custom Path'), 'C:\\Windows\\Temp\\bad');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/Custom Path rejected/i);

    // Custom path add
    const input = screen.getByLabelText('Add Custom Path');
    await user.clear(input);
    await user.type(input, 'C:\\Users\\Alice\\CustomCleanup');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));
    await screen.findByText(/Settings saved/i);
    expect(await screen.findByText('C:\\Users\\Alice\\CustomCleanup')).toBeInTheDocument();

    // Remove
    await user.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(screen.queryByText('C:\\Users\\Alice\\CustomCleanup')).not.toBeInTheDocument());
  });

  it('SettingsView shows Recycle Bin as Deep Cleanup sub-item and exposes neutral custom path preset', async () => {
    const user = userEvent.setup();
    render(
      <SettingsView
        client={mockController}
        settings={baseSettings()}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onSaved={() => {}}
      />,
    );

    const recycleNote = await screen.findByLabelText('Recycle Bin included with Deep Cleanup');
    function expectRecycleBinAndDeepControls() {
      expect(recycleNote).toHaveTextContent('Controlled by Deep Cleanup');
      const recycleBin = screen.queryByLabelText(/Empty Windows Recycle Bin/i);
      expect(recycleBin).not.toBeInTheDocument();
      expect(screen.getByLabelText('Enable deep cleanup categories')).toBeInTheDocument();
      const staleDays = screen.getByLabelText('Inactive project cache age');
      expect(staleDays).toBeDisabled();
    }
    expectRecycleBinAndDeepControls();

    const usePreset = screen.getByRole('button', { name: 'Use example custom path preset' });
    await user.click(usePreset);
    const customPathInput = screen.getByLabelText('Add Custom Path') as HTMLInputElement;
    expect(customPathInput.value).toMatch(/^C:\\Temp\\/);
    expect(customPathInput.value.toLowerCase()).not.toContain('\\users\\');
  });

  it('keeps unsaved settings draft when onSaved identity changes', async () => {
    const settings = baseSettings();
    const { rerender } = render(<SettingsView client={mockController} settings={settings} elevationStatus={{ isElevated: false }} elevationError={null} onSaved={() => {}} />);
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    fireEvent.change(minAge, { target: { value: '12' } });
    rerender(<SettingsView client={mockController} settings={settings} elevationStatus={{ isElevated: false }} elevationError={null} onSaved={() => {}} />);
    expect((screen.getByLabelText('Minimum file age') as HTMLInputElement).value).toBe('12');
  });

  it('preserves unrelated draft edits while custom path validation is pending', async () => {
    const user = userEvent.setup();
    const pending = deferred<any>();
    const client = { ...mockController, validateCustomPath: vi.fn(() => pending.promise), updateSettings: vi.fn(async (s: Settings) => s) } as any;
    render(<SettingsView client={client} settings={baseSettings()} elevationStatus={{ isElevated: false }} elevationError={null} onSaved={() => {}} />);
    await user.type(screen.getByLabelText('Add Custom Path'), 'C:\\Temp\\MyCache');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    pending.resolve({ allowed: true, path: 'C:\\Temp\\MyCache', reason: null });
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalled());
    const saved = client.updateSettings.mock.calls[0][0] as Settings;
    expect(saved.customPaths).toContain('C:\\Temp\\MyCache');
    expect(saved.minAgeHours).toBe(12);
  });

  it('preserves post-submit edits while updateSettings is in flight', async () => {
    const user = userEvent.setup();
    const pending = deferred<Settings>();
    const client = { ...mockController, updateSettings: vi.fn(() => pending.promise) } as any;
    render(<SettingsView client={client} settings={baseSettings()} elevationStatus={{ isElevated: false }} elevationError={null} onSaved={() => {}} />);
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    const allow = screen.getByLabelText('Allow deleting unknown folders (advanced)') as HTMLInputElement;
    await user.click(allow);
    pending.resolve({ ...baseSettings(), minAgeHours: 12 });
    await waitFor(() => expect(screen.getByText(/Settings saved/i)).toBeInTheDocument());
    expect((screen.getByLabelText('Allow deleting unknown folders (advanced)') as HTMLInputElement).checked).toBe(true);
  });
});
