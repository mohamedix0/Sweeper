import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';
import { fakeCloseBridge as fakeBridge } from './close/fakeCloseBridge';

type User = ReturnType<typeof userEvent.setup>;

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
