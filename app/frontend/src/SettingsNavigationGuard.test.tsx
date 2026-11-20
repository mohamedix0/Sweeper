import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';

type User = ReturnType<typeof userEvent.setup>;

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
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

describe('Settings navigation guard (#91)', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('navigates without prompting when the Settings draft has no changes', async () => {
    const { user } = setup();
    await openSettings(user);

    expect(screen.getByText('All changes saved')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));

    expect(await screen.findByRole('heading', { name: 'Scan & Clean' })).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('prompts on a dirty draft and keeps the edit when the user stays', async () => {
    const { user } = setup();
    await openSettings(user);
    await makeTier1Dirty(user);

    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));

    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent(/unsaved/i);
    // Navigation is held: Settings is still the visible view.
    expect(screen.getByRole('heading', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Scan & Clean' })).not.toBeInTheDocument();

    await user.click(within(dialog).getByRole('button', { name: 'Keep editing' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(tier1Checkbox().checked).toBe(false);
  });

  it('discards the draft only when the user explicitly chooses Discard changes', async () => {
    const { user } = setup();
    await openSettings(user);
    await makeTier1Dirty(user);

    await user.click(screen.getByRole('button', { name: 'Activity' }));
    const dialog = await screen.findByRole('alertdialog');
    await user.click(within(dialog).getByRole('button', { name: 'Discard changes' }));

    expect(await screen.findByRole('heading', { name: 'Activity History' })).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();

    // Returning to Settings shows the saved value: the unsaved draft was discarded.
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    expect(tier1Checkbox().checked).toBe(true);
  });
});
