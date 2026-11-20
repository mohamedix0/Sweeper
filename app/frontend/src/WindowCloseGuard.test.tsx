import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import type { Settings, SweeperClient } from './api/client';
import { mockController } from './api/mock';
import { fakeCloseBridge as fakeBridge } from './close/fakeCloseBridge';

type User = ReturnType<typeof userEvent.setup>;

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => { resolve = res; });
  return { promise, resolve };
}

async function openSettings(user: User) {
  await user.click(screen.getByRole('button', { name: 'Settings' }));
  await screen.findByRole('heading', { name: 'Settings' });
}

function tier1Checkbox(): HTMLInputElement {
  return screen.getByLabelText('Enable Tier 1') as HTMLInputElement;
}

async function makeTier1Dirty(user: User) {
  expect(tier1Checkbox().checked).toBe(true);
  await user.click(tier1Checkbox());
  expect(tier1Checkbox().checked).toBe(false);
}

describe('window close with an unsaved Settings draft (#93)', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('closes normally with no prompt when there is no dirty draft', () => {
    const bridge = fakeBridge();
    render(<App closeBridge={bridge} />);

    act(() => bridge.requestHostClose());

    expect(bridge.decisions).toEqual([true]);
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('warns on a dirty close and keeps the app open when cancelled', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    render(<App closeBridge={bridge} />);
    await openSettings(user);
    await makeTier1Dirty(user);

    act(() => bridge.requestHostClose());

    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent(/unsaved/i);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    expect(bridge.decisions).toEqual([false]);
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(screen.getByRole('heading', { name: 'Settings' })).toBeInTheDocument();
    expect(tier1Checkbox().checked).toBe(false);
  });

  it('settles pending Add Folder validation and its save before approving close (#93)', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    type Verdict = Awaited<ReturnType<SweeperClient['validateCustomPath']>>;
    const validation = deferred<Verdict>();
    const savedPaths: string[][] = [];
    vi.spyOn(mockController, 'validateCustomPath').mockImplementation(() => validation.promise);
    vi.spyOn(mockController, 'updateSettings').mockImplementation(async (s: Settings) => {
      savedPaths.push([...s.customPaths]);
      return s;
    });

    render(<App closeBridge={bridge} />);
    await openSettings(user);

    // Start Add Folder; its validation is still pending when close is asked.
    await user.type(screen.getByLabelText('Add Custom Path'), 'C:\\Temp\\CloseSafe');
    await user.click(screen.getByRole('button', { name: 'Add Folder' }));

    act(() => bridge.requestHostClose('R1'));
    const dialog = await screen.findByRole('alertdialog');
    expect(bridge.decisions).toEqual([]);

    await user.click(within(dialog).getByRole('button', { name: 'Save & close' }));
    // Approval is held while validation — and the save it will queue — settle.
    expect(bridge.decisions).toEqual([]);

    validation.resolve({ allowed: true, path: 'C:\\Temp\\CloseSafe', reason: 'ok' });

    await waitFor(() => expect(bridge.decisions).toEqual([true]));
    // The path actually reached the backend before the close was approved.
    expect(savedPaths).toContainEqual(['C:\\Temp\\CloseSafe']);
  });

  it('persists the draft before approving a dirty close so no edit is lost', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    render(<App closeBridge={bridge} />);
    await openSettings(user);
    await makeTier1Dirty(user);

    act(() => bridge.requestHostClose());
    const dialog = await screen.findByRole('alertdialog');
    await user.click(within(dialog).getByRole('button', { name: 'Save & close' }));

    await waitFor(() => expect(bridge.decisions).toEqual([true]));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());

    // The edit was actually saved, not merely acknowledged by the UI.
    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
    await openSettings(user);
    expect(tier1Checkbox().checked).toBe(false);
  });
});
