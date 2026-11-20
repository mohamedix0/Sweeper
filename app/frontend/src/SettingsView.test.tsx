import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef, useCallback, useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { SettingsView } from './components/SettingsView';
import { useSettingsController } from './components/settings/useSettingsController';
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
});
