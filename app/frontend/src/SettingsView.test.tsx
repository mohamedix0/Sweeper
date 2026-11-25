import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef, useCallback, useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { SettingsView } from './components/SettingsView';
import {
  isPendingDraftDirty,
  useSettingsController,
} from './components/settings/useSettingsController';
import { mockController } from './api/mock';
import type { Settings, SweeperClient } from './api/client';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (r?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
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
    await user.type(input, 'C:\\Temp\\CustomCleanup\\Cache');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));
    await screen.findByText(/Settings saved/i);
    expect(await screen.findByText('C:\\Temp\\CustomCleanup\\Cache')).toBeInTheDocument();

    // Remove
    await user.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(screen.queryByText('C:\\Temp\\CustomCleanup\\Cache')).not.toBeInTheDocument());
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

  it('does not leave a false-dirty draft when parent settings arrive before the self-load resolves (#136)', async () => {
    const parentValue = baseSettings();
    const selfLoad = deferred<Settings>();
    const client: SweeperClient = { ...mockController, getSettings: vi.fn(() => selfLoad.promise) };

    function Harness() {
      const [settings, setSettings] = useState<Settings | null>(null);
      const { draft, isDirty } = useSettingsController({ client, settings, onSaved: () => {} });
      return (
        <>
          <output data-testid="draft-min-age">{draft?.minAgeHours ?? 'none'}</output>
          <output data-testid="dirty">{String(isDirty)}</output>
          <button onClick={() => setSettings(parentValue)}>parent-load</button>
        </>
      );
    }

    const user = userEvent.setup();
    render(<Harness />);
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('none');

    // The parent's boot load commits while the controller's self-load is still
    // pending. Adopting the parent value must also acknowledge it, otherwise
    // draft = s / acknowledged = null leaves the draft dirty forever (#136).
    await user.click(screen.getByRole('button', { name: 'parent-load' }));
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('24');

    // The cancelled self-load resolving later must not revive a stale draft.
    selfLoad.resolve({ ...parentValue, minAgeHours: 12 });
    await waitFor(() => expect(screen.getByTestId('draft-min-age')).toHaveTextContent('24'));
    expect(screen.getByTestId('dirty')).toHaveTextContent('false');
  });

  it('does not show the unsaved-settings dialog after the boot load beats the controller self-load (#136)', async () => {
    const parentValue = baseSettings();
    const parentLoad = deferred<Settings>();
    const selfLoad = deferred<Settings>();
    let calls = 0;
    const getSettings = vi.fn(() => {
      calls += 1;
      if (calls === 1) return parentLoad.promise;
      if (calls === 2) return selfLoad.promise;
      return Promise.resolve(parentValue);
    });
    const originalGetSettings = mockController.getSettings;
    (mockController as SweeperClient).getSettings = getSettings;
    try {
      const user = userEvent.setup();
      const view = render(<App />);

      await user.click(screen.getByRole('button', { name: 'Settings' }));
      await screen.findByText('Loading settings…');

      // App's boot load lands first while the controller's self-load is still
      // pending, exercising the adoption path that used to skip acknowledgement.
      await act(async () => { parentLoad.resolve(parentValue); });
      await screen.findByLabelText('Minimum file age');

      // The cancelled self-load lands later with a different value; it must not
      // turn the adopted, clean draft dirty.
      await act(async () => { selfLoad.resolve({ ...parentValue, minAgeHours: 12 }); });

      await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
      expect(screen.queryByText('Unsaved settings changes')).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: /keep editing/i })).not.toBeInTheDocument();
      view.unmount();
    } finally {
      (mockController as SweeperClient).getSettings = originalGetSettings;
    }
  });

  it('preserves unrelated draft edits while custom path validation is pending', async () => {
    const user = userEvent.setup();
    type ValidateResult = Awaited<ReturnType<SweeperClient['validateCustomPath']>>;
    const pending = deferred<ValidateResult>();
    const updateSettings = vi.fn(async (s: Settings) => s);
    const validateCustomPath = vi.fn(() => pending.promise);
    const client: SweeperClient = { ...mockController, validateCustomPath, updateSettings };
    render(<SettingsView client={client} settings={baseSettings()} elevationStatus={{ isElevated: false }} elevationError={null} onSaved={() => {}} />);
    await user.type(screen.getByLabelText('Add Custom Path'), 'C:\\Temp\\MyCache');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    pending.resolve({ allowed: true, path: 'C:\\Temp\\MyCache', reason: 'ok' });
    await waitFor(() => expect(updateSettings).toHaveBeenCalled());
    const saved = updateSettings.mock.calls[0][0];
    expect(saved.customPaths).toContain('C:\\Temp\\MyCache');
    expect(saved.minAgeHours).toBe(12);
  });

  it('preserves post-submit edits while updateSettings is in flight', async () => {
    const user = userEvent.setup();
    const pending = deferred<Settings>();
    const updateSettings = vi.fn(() => pending.promise);
    const client: SweeperClient = { ...mockController, updateSettings };
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

  it('single-flight saves converge on one consistent final state across overlapping saves', async () => {
    type SaveCall = { submitted: Settings; resolve: (value: Settings) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (value: Settings) => void;
      const promise = new Promise<Settings>((r) => { resolve = r; });
      calls.push({ submitted, resolve });
      return promise;
    });
    const onSaved = vi.fn();
    const client: SweeperClient = { ...mockController, updateSettings };

    // A harness that exposes save() directly: overlapping saves can originate
    // from any caller (Save button, custom-path actions, auto-save), not just
    // the button which is disabled while `saving` is true.
    const initialSettings = baseSettings();
    const first = { ...initialSettings, minAgeHours: 12 };
    const second = { ...initialSettings, minAgeHours: 12, allowDeleteUnknown: true };
    function SaveHarness() {
      const { save } = useSettingsController({
        client,
        settings: initialSettings,
        onSaved,
      });
      return (
        <>
          <button onClick={() => void save(first)}>save-first</button>
          <button onClick={() => void save(second)}>save-second</button>
        </>
      );
    }
    const user = userEvent.setup();
    render(<SaveHarness />);

    // First save stays in flight.
    await user.click(screen.getByRole('button', { name: 'save-first' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);
    expect(calls[0].submitted).toEqual(first);

    // A second save is requested while the first is still in flight.
    await user.click(screen.getByRole('button', { name: 'save-second' }));

    // Single-flight: the second save is queued behind the first, not started.
    expect(updateSettings).toHaveBeenCalledTimes(1);

    // Resolve the first save; the queued save must now start.
    calls[0].resolve(first);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));

    // The queued save carries the latest user intent, so the backend converges
    // on one consistent final state.
    expect(calls[1].submitted).toEqual(second);

    // Resolve the second save. The final UI state must match what was persisted.
    // Require the second onSaved callback specifically: mergePreservingEdits can
    // already produce the second submission's value from the first callback, so
    // merely waiting for any onSaved call would not prove the queued save ran.
    calls[1].resolve(second);
    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(2));
    const finalSaved = onSaved.mock.calls[onSaved.mock.calls.length - 1][0] as Settings;
    expect(finalSaved).toEqual(second);
  });

  it('queued saves from an unmounted controller do not overwrite a newer save from a remounted controller', async () => {
    type SaveCall = { submitted: Settings; resolve: (value: Settings) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (value: Settings) => void;
      const promise = new Promise<Settings>((r) => { resolve = r; });
      calls.push({ submitted, resolve });
      return promise;
    });
    const onSaved = vi.fn();
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();
    const first = { ...initial, minAgeHours: 11 };
    const second = { ...initial, minAgeHours: 12 };
    const newest = { ...initial, minAgeHours: 13 };

    function Harness({ firstSave, secondSave }: { firstSave: Settings; secondSave?: Settings }) {
      const { save } = useSettingsController({ client, settings: initial, onSaved });
      return (
        <>
          <button onClick={() => void save(firstSave)}>save-first</button>
          {secondSave && <button onClick={() => void save(secondSave)}>save-second</button>}
        </>
      );
    }

    const user = userEvent.setup();
    const firstMount = render(<Harness firstSave={first} secondSave={second} />);

    // The first save is in flight and the second is queued behind it.
    await user.click(screen.getByRole('button', { name: 'save-first' }));
    await user.click(screen.getByRole('button', { name: 'save-second' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);

    // Unmount with a save still queued in the old controller.
    firstMount.unmount();

    // A remounted controller submits a newer save. The shared queue keeps it
    // behind the old controller's saves, so it must still be the last write.
    render(<Harness firstSave={newest} />);
    await user.click(screen.getByRole('button', { name: 'save-first' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);

    calls[0].resolve(first);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    calls[1].resolve(second);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(3));

    // Submissions stay in global order: old controller's saves, then the new one.
    expect(calls.map((c) => c.submitted.minAgeHours)).toEqual([11, 12, 13]);

    calls[2].resolve(newest);
    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(3));
    const finalSaved = onSaved.mock.calls[onSaved.mock.calls.length - 1][0] as Settings;
    expect(finalSaved).toEqual(newest);
  });

  it('shows the last submitted value after queued saves settle despite stale parent feedback on remount', async () => {
    type SaveCall = { submitted: Settings; resolve: (value: Settings) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (value: Settings) => void;
      const promise = new Promise<Settings>((r) => { resolve = r; });
      calls.push({ submitted, resolve });
      return promise;
    });
    const onSaved = vi.fn();
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();
    const first = { ...initial, minAgeHours: 11 };
    const second = { ...initial, minAgeHours: 12 };
    const newest = { ...initial, minAgeHours: 13 };

    function Controller({ settings, onSave }: { settings: Settings | null; onSave: (s: Settings) => void }) {
      const { draft, save } = useSettingsController({ client, settings, onSaved: onSave });
      return (
        <>
          <output data-testid="draft-min-age">{draft?.minAgeHours ?? 'none'}</output>
          <button onClick={() => void save(first)}>save-first</button>
          <button onClick={() => void save(second)}>save-second</button>
          <button onClick={() => void save(newest)}>save-newest</button>
        </>
      );
    }

    // The App keeps one settings state across a controller remount and feeds
    // every onSaved value back in as the controller's `settings` prop.
    function AppHarness({ phase }: { phase: 1 | 2 }) {
      const [settings, setSettings] = useState<Settings | null>(initial);
      const handleSaved = useCallback((s: Settings) => { onSaved(s); setSettings(s); }, []);
      return <Controller key={phase} settings={settings} onSave={handleSaved} />;
    }

    const user = userEvent.setup();
    const view = render(<AppHarness phase={1} />);

    // Two saves from the first controller: one in flight, one queued.
    await user.click(screen.getByRole('button', { name: 'save-first' }));
    await user.click(screen.getByRole('button', { name: 'save-second' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);

    // Remount the controller with the App state intact and submit a newer save.
    view.rerender(<AppHarness phase={2} />);
    await user.click(screen.getByRole('button', { name: 'save-newest' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);

    // Resolve in submission order. Save 11's feedback carries minAge 12 back
    // into the remounted controller through the parent `settings` prop while
    // saves 12 and 13 are still outstanding.
    calls[0].resolve(first);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    calls[1].resolve(second);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(3));

    calls[2].resolve(newest);
    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(3));

    expect(calls.map((c) => c.submitted.minAgeHours)).toEqual([11, 12, 13]);
    const finalSaved = onSaved.mock.calls[onSaved.mock.calls.length - 1][0] as Settings;
    expect(finalSaved).toEqual(newest);
    // The UI must end on the last submitted value, not the stale minAge 12
    // delivered as parent feedback from the earlier save.
    await waitFor(() => expect(screen.getByTestId('draft-min-age')).toHaveTextContent('13'));
  });

  it('replays deferred parent settings when the final queued save rejects after remount', async () => {
    // Bundle the controllable saves, callback spy and settings values into a
    // single fixture; the two saves return their promises in call order.
    const fixture = (() => {
      const saves = [deferred<Settings>(), deferred<Settings>()];
      let index = 0;
      const updateSettings = vi.fn(() => saves[index++].promise);
      const onSaved = vi.fn();
      const initial = baseSettings();
      const client: SweeperClient = { ...mockController, updateSettings };
      return { saves, onSaved, initial, queued: { ...initial, minAgeHours: 11 }, client };
    })();

    function Controller({ settings, onSave }: { settings: Settings | null; onSave: (s: Settings) => void }) {
      const { draft, save } = useSettingsController({ client: fixture.client, settings, onSaved: onSave });
      return (
        <>
          <output data-testid="draft-min-age">{draft?.minAgeHours ?? 'none'}</output>
          <button onClick={() => void save(fixture.queued)}>save-queued</button>
        </>
      );
    }

    // The App keeps one settings state across a controller remount and feeds
    // every onSaved value back in as the controller's `settings` prop.
    function AppHarness({ phase }: { phase: 1 | 2 }) {
      const [settings, setSettings] = useState<Settings | null>(fixture.initial);
      const handleSaved = useCallback((s: Settings) => { fixture.onSaved(s); setSettings(s); }, []);
      return <Controller key={phase} settings={settings} onSave={handleSaved} />;
    }

    const user = userEvent.setup();
    const view = render(<AppHarness phase={1} />);

    // Queue two identical saves of minAge 11 from the first controller: one in
    // flight, one queued behind it.
    await user.click(screen.getByRole('button', { name: 'save-queued' }));
    await user.click(screen.getByRole('button', { name: 'save-queued' }));
    expect(fixture.client.updateSettings).toHaveBeenCalledTimes(1);

    // Remount the controller with the parent still holding the original value
    // (24). With saves outstanding, the controller keeps showing 24.
    view.rerender(<AppHarness phase={2} />);
    await screen.findByTestId('draft-min-age');
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('24');

    // The first save succeeds: parent/backend move to 11, but its feedback is
    // suppressed while the second save is still pending.
    fixture.saves[0].resolve(fixture.queued);
    await waitFor(() => expect(fixture.onSaved).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(fixture.client.updateSettings).toHaveBeenCalledTimes(2));
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('24');

    // The final save rejects, so no onSaved follows and the parent value (11)
    // never changes again. The queue-drain replay must still reconcile the
    // remounted controller to the persisted parent/backend value.
    fixture.saves[1].reject(new Error('updateSettings failed'));
    await waitFor(() => expect(screen.getByTestId('draft-min-age')).toHaveTextContent('11'));
    expect(fixture.onSaved).toHaveBeenCalledTimes(1);
  });

  it('keeps a rejected local submission when the parent value never changed during the pending save', async () => {
    // Same controller, parent holds 24 the whole time. The user submits 12 and
    // the save rejects, so no onSaved fires and there is no deferred parent
    // feedback. The queue-drain replay must not reconcile the rejected 12 back
    // to the unchanged parent value of 24.
    const pending = deferred<Settings>();
    const updateSettings = vi.fn(() => pending.promise);
    const onSaved = vi.fn();
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();

    function Controller() {
      const { draft, error, save } = useSettingsController({ client, settings: initial, onSaved });
      return (
        <>
          <output data-testid="draft-min-age">{draft?.minAgeHours ?? 'none'}</output>
          <output data-testid="error">{error ?? 'none'}</output>
          <button onClick={() => void save({ ...initial, minAgeHours: 12 })}>save-rejected</button>
        </>
      );
    }

    const user = userEvent.setup();
    render(<Controller />);
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('24');

    await user.click(screen.getByRole('button', { name: 'save-rejected' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('12');

    pending.reject(new Error('updateSettings failed'));
    // The error stays visible and the rejected input stays in the draft: there
    // was no changed parent value to replay when the queue drained.
    await waitFor(() => expect(screen.getByTestId('error')).not.toHaveTextContent('none'));
    expect(screen.getByTestId('draft-min-age')).toHaveTextContent('12');
    expect(onSaved).not.toHaveBeenCalled();
  });

  // --- Elevate persists the pending draft before restart (#92) ------------------

  it('persists the pending draft before the Settings Elevate restarts', async () => {
    const user = userEvent.setup();
    const events: string[] = [];
    const stored = { ...baseSettings() };
    const client: SweeperClient = {
      ...mockController,
      updateSettings: vi.fn(async (s: Settings) => {
        events.push('updateSettings');
        Object.assign(stored, s);
        return s;
      }),
      relaunchElevated: vi.fn(async () => {
        events.push('relaunchElevated');
        return { launched: true };
      }),
      getSettings: vi.fn(async () => ({ ...stored })),
    };
    render(
      <SettingsView
        client={client}
        settings={baseSettings()}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onSaved={() => {}}
      />,
    );

    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    await user.click(screen.getByLabelText('Run as administrator'));

    await waitFor(() => expect(events).toContain('relaunchElevated'));
    expect(events[0], 'the draft must be saved before the restart request').toBe('updateSettings');
    expect(stored.minAgeHours).toBe(12);
  });

  it('persists the pending draft before the sidebar Elevate restarts', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });

    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    await user.click(screen.getByLabelText('Standard Mode · Elevate'));

    await waitFor(async () => {
      const persisted = await mockController.getSettings();
      expect(persisted.minAgeHours).toBe(12);
    });
  });

  it('flushes a draft edit made while an earlier save is still in flight', async () => {
    type SaveCall = { submitted: Settings; resolve: (value: Settings) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (value: Settings) => void;
      const promise = new Promise<Settings>((r) => { resolve = r; });
      calls.push({ submitted, resolve });
      return promise;
    });
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();

    function Harness() {
      const { draft, setDraft, isDirty, save, flushDraft } = useSettingsController({
        client, settings: initial, onSaved: () => {},
      });
      return (
        <>
          <output data-testid="draft-min-age">{draft?.minAgeHours ?? 'none'}</output>
          <output data-testid="dirty">{String(isDirty)}</output>
          <button onClick={() => void save({ ...initial, minAgeHours: 12 })}>save-12</button>
          <button onClick={() => setDraft((d) => (d ? { ...d, minAgeHours: 6 } : d))}>edit-6</button>
          <button onClick={() => void flushDraft()}>flush</button>
        </>
      );
    }
    const user = userEvent.setup();
    render(<Harness />);

    // Save 12 starts; while it is in flight the user edits the draft to 6.
    await user.click(screen.getByRole('button', { name: 'save-12' }));
    await user.click(screen.getByRole('button', { name: 'edit-6' }));
    expect(updateSettings).toHaveBeenCalledTimes(1);
    expect(calls[0].submitted.minAgeHours).toBe(12);

    // The backend acknowledged 12, but the merged draft keeps the newer edit.
    // The draft must stay dirty against the ACKNOWLEDGED value (12), not the
    // parent feedback that carries the merged edit.
    calls[0].resolve(calls[0].submitted);
    await waitFor(() => expect(screen.getByTestId('draft-min-age')).toHaveTextContent('6'));
    await waitFor(() => expect(screen.getByTestId('dirty')).toHaveTextContent('true'));

    // Flush persists the newer edit instead of skipping it as already saved.
    await user.click(screen.getByRole('button', { name: 'flush' }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    expect(calls[1].submitted.minAgeHours).toBe(6);
    calls[1].resolve(calls[1].submitted);
  });

  it('never declares a draft clean while a save of a different value is outstanding', async () => {
    type SaveCall = { submitted: Settings; resolve: (value: Settings) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (value: Settings) => void;
      const promise = new Promise<Settings>((r) => { resolve = r; });
      calls.push({ submitted, resolve });
      return promise;
    });
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();
    let flushResult: boolean | null = null;

    function Harness() {
      const controller = useSettingsController({ client, settings: initial, onSaved: () => {} });
      return (
        <>
          <output data-testid="draft">{controller.draft?.minAgeHours ?? 'none'}</output>
          <button onClick={() => void controller.save({ ...initial, minAgeHours: 12 })}>save-12</button>
          <button onClick={() => controller.setDraft((d) => (d ? { ...d, minAgeHours: 24 } : d))}>edit-24</button>
          <button onClick={() => { void controller.flushDraft().then((ok) => { flushResult = ok; }); }}>flush</button>
        </>
      );
    }
    const user = userEvent.setup();
    render(<Harness />);

    // Save 12 is outstanding, then the draft is edited back to the currently
    // acknowledged value 24. Ref-only equality would call this clean even
    // though the backend is about to persist 12 (#93).
    await user.click(screen.getByRole('button', { name: 'save-12' }));
    await user.click(screen.getByRole('button', { name: 'edit-24' }));
    expect(screen.getByTestId('draft')).toHaveTextContent('24');
    expect(isPendingDraftDirty()).toBe(true);

    // The in-flight save lands as 12 while the draft keeps the newer edit 24.
    calls[0].resolve(calls[0].submitted);
    await waitFor(() => expect(isPendingDraftDirty()).toBe(true));

    // Flushing must not skip the corrective save just because the draft
    // momentarily matched the pre-save acknowledgement.
    await user.click(screen.getByRole('button', { name: 'flush' }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    expect(calls[1].submitted.minAgeHours).toBe(24);
    calls[1].resolve(calls[1].submitted);
    await waitFor(() => expect(flushResult).toBe(true));
  });

  it('flushDraft stays false when a save queued behind the flush fails (#93)', async () => {
    type SaveCall = { submitted: Settings; resolve: (v: Settings) => void; reject: (e?: unknown) => void };
    const calls: SaveCall[] = [];
    const updateSettings = vi.fn((submitted: Settings) => {
      let resolve!: (v: Settings) => void;
      let reject!: (e?: unknown) => void;
      const promise = new Promise<Settings>((res, rej) => { resolve = res; reject = rej; });
      calls.push({ submitted, resolve, reject });
      return promise;
    });
    const client: SweeperClient = { ...mockController, updateSettings };
    const initial = baseSettings();
    let flushResult: boolean | null = null;

    function Harness() {
      const c = useSettingsController({ client, settings: initial, onSaved: () => {} });
      return (
        <>
          <button onClick={() => c.setDraft((d) => (d ? { ...d, minAgeHours: 12 } : d))}>edit-12</button>
          <button onClick={() => void c.save({ ...initial, minAgeHours: 24 })}>save-24</button>
          <button onClick={() => { void c.flushDraft().then((ok) => { flushResult = ok; }); }}>flush</button>
        </>
      );
    }
    const user = userEvent.setup();
    render(<Harness />);

    // Make the draft dirty, then start a flush so our own save is in flight.
    await user.click(screen.getByRole('button', { name: 'edit-12' }));
    await user.click(screen.getByRole('button', { name: 'flush' }));
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(calls[0].submitted.minAgeHours).toBe(12);

    // A save requested while the flush's save is in flight queues behind it.
    await user.click(screen.getByRole('button', { name: 'save-24' }));
    expect(updateSettings).toHaveBeenCalledTimes(1); // still queued

    // The flush's own save succeeds; the save queued behind it then fails.
    calls[0].resolve(calls[0].submitted);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(2));
    calls[1].reject(new Error('disk write failed'));

    await waitFor(() => expect(flushResult).not.toBeNull());
    // The draft holds 24 but the backend only acknowledged 12, so the close
    // prompt must stay open: flush reports false, not just its own result.
    expect(flushResult).toBe(false);
  });

  it('aborts Elevate when the draft cannot be persisted', async () => {
    const relaunchElevated = vi.fn(async () => ({ launched: true }));
    const updateSettings = vi.fn(async (_s: Settings): Promise<Settings> => {
      throw new Error('disk write failed');
    });
    const client: SweeperClient = { ...mockController, updateSettings, relaunchElevated };
    render(
      <SettingsView
        client={client}
        settings={baseSettings()}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onSaved={() => {}}
      />,
    );

    const user = userEvent.setup();
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    fireEvent.change(minAge, { target: { value: '12' } });
    expect((screen.getByLabelText('Minimum file age') as HTMLInputElement).value).toBe('12');
    await user.click(screen.getByLabelText('Run as administrator'));

    await waitFor(() => expect(updateSettings).toHaveBeenCalledTimes(1));
    expect(relaunchElevated).not.toHaveBeenCalled();
    expect(await screen.findByText(/could not be saved/i)).toBeInTheDocument();
    expect((screen.getByLabelText('Minimum file age') as HTMLInputElement).value).toBe('12');
  });

  it('locks every draft-bound control until the Elevate request settles', async () => {
    type Relaunch = Awaited<ReturnType<SweeperClient['relaunchElevated']>>;
    const pending = deferred<Relaunch>();
    const relaunchElevated = vi.fn(() => pending.promise);
    const client: SweeperClient = {
      ...mockController,
      updateSettings: vi.fn(async (s: Settings) => s),
      relaunchElevated,
    };
    render(
      <SettingsView
        client={client}
        settings={baseSettings()}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onSaved={() => {}}
      />,
    );

    const user = userEvent.setup();
    const minAge = screen.getByLabelText('Minimum file age') as HTMLInputElement;
    await user.clear(minAge);
    await user.type(minAge, '12');
    await user.click(screen.getByLabelText('Run as administrator'));

    await waitFor(() => expect(relaunchElevated).toHaveBeenCalledTimes(1));
    expect(minAge).toBeDisabled();
    expect(screen.getByLabelText('Enable Tier 1')).toBeDisabled();
    expect(screen.getByLabelText('Enable Tier 2')).toBeDisabled();
    expect(screen.getByLabelText('Enable deep cleanup categories')).toBeDisabled();
    expect(screen.getByLabelText('Allow deleting unknown folders (advanced)')).toBeDisabled();
    expect(screen.getByLabelText('Add Custom Path')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Enable Deep Cleanup' })).toBeDisabled();
    expect(screen.getByLabelText('Run as administrator')).toBeDisabled();

    pending.resolve({ launched: true });
    await waitFor(() => expect(minAge).toBeEnabled());
    expect(screen.getByLabelText('Enable Tier 1')).toBeEnabled();
  });
});
