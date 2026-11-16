import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

async function openAnalyzer(input: { user: ReturnType<typeof userEvent.setup> }) {
  await input.user.click(screen.getByRole('button', { name: 'Analyzer' }));
  await input.user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
}

async function findSelectableUnknownCheckbox() {
  const items = screen.getAllByRole('treeitem');
  for (const item of items) {
    const text = item.textContent ?? '';
    const isUnknown = /Unknown/i.test(text);
    if (!isUnknown) continue;
    const cb = within(item as HTMLElement).queryByRole('checkbox') as HTMLInputElement | null;
    if (cb && !cb.disabled) return cb;
  }
  return null;
}

async function enableUnknownInSettings(input: { user: ReturnType<typeof userEvent.setup> }) {
  await input.user.click(screen.getByRole('button', { name: 'Settings' }));
  await screen.findByRole('heading', { name: 'Settings' });
  await input.user.click(screen.getByLabelText('Allow deleting unknown folders (advanced)'));
  await input.user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
  await screen.findByText(/Settings saved/i);
}


async function runGatingTest() {
  const { user } = setup();
  await openAnalyzer({ user });
  const statuses = await screen.findAllByText(/Unknown gated by Settings/i);
  const row = statuses[0]?.closest('[role="treeitem"]') as HTMLElement | null;
  expect(row).not.toBeNull();
  const unknownCb = within(row!).getByRole('checkbox') as HTMLInputElement;
  expect(unknownCb).toBeDisabled();
  await enableUnknownInSettings({ user });
  await openAnalyzer({ user });
  const unknownCb2 = await findSelectableUnknownCheckbox();
  expect(unknownCb2).not.toBeNull();
  await user.click(unknownCb2!);
  expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
  const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
  expect(previewBtn).toBeDisabled();
  await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
  expect(previewBtn).toBeEnabled();
}

async function runNonSelectableTest() {
  const { user } = setup();
  await openAnalyzer({ user });
  const all = screen.getAllByRole('checkbox') as HTMLInputElement[];
  const findBySuffix = (input: { suffix: string }) => all.find((n) => String(n.getAttribute('aria-label') ?? '').includes(input.suffix));
  const vhdx = findBySuffix({ suffix: 'dev-machine.vhdx' });
  expect(vhdx).toBeTruthy();
  expect(vhdx!).toBeDisabled();
  expect(vhdx!.closest('[role="treeitem"]')).toHaveTextContent(/\.vhdx/i);
  const gitNode = findBySuffix({ suffix: '\\Dev\\repo\\.git' });
  expect(gitNode!).toBeDisabled();
  const linkNode = findBySuffix({ suffix: '\\Users\\Alice\\OneDriveLink' });
  expect(linkNode!).toBeDisabled();
}

async function runStickyBarTest() {
  const { user } = setup();
  await openAnalyzer({ user });
  const previewBtn = screen.getByRole('button', { name: 'Review & Clean' }) as HTMLButtonElement;
  expect(previewBtn).toBeDisabled();
  const safeNode = await screen.findByLabelText(/Select analyzer node .*AppData\\Local\\Temp$/);
  await user.click(safeNode);
  expect(previewBtn).toBeEnabled();
  expect(previewBtn.className).toMatch(/primary/);
}

describe('Analyzer app flows', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('Analyzer gates Unknown selection behind allow_unknown and requires typed confirmation for Unknown nodes', async () => {
    await runGatingTest();
  });

  it('marks non-selectable analyzer nodes (.vhdx / Protected / Link Point) correctly', async () => {
    await runNonSelectableTest();
  });

  it('shows Analyzer sticky action bar Review & Clean with disabled/enabled states', async () => {
    await runStickyBarTest();
  });
});
