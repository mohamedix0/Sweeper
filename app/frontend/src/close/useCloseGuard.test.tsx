import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CloseDraftDialog } from '../components/CloseDraftDialog';
import type { CloseBridge } from './closeBridge';
import { useCloseGuard, type CloseGuardDeps } from './useCloseGuard';

type FakeBridge = CloseBridge & {
  decisions: boolean[];
  requestHostClose: () => void;
};

function fakeBridge(): FakeBridge {
  const listeners: Array<() => void> = [];
  const decisions: boolean[] = [];
  return {
    decisions,
    onHostCloseRequest(handler) {
      listeners.push(handler);
      return () => {
        const index = listeners.indexOf(handler);
        if (index >= 0) listeners.splice(index, 1);
      };
    },
    async resolveClose(approved) {
      decisions.push(approved);
    },
    requestHostClose() {
      for (const listener of [...listeners]) listener();
    },
  };
}

function Harness({ deps }: { deps: CloseGuardDeps }) {
  const guard = useCloseGuard(deps);
  return (
    <div>
      <output data-testid="prompt">{String(guard.promptOpen)}</output>
      {guard.promptOpen && (
        <CloseDraftDialog
          saving={guard.saving}
          error={guard.error}
          onCancel={guard.cancel}
          onSaveAndClose={() => void guard.saveAndClose()}
        />
      )}
    </div>
  );
}

function depsOf(bridge: CloseBridge, dirty: boolean, flush: () => Promise<boolean>): CloseGuardDeps {
  return { bridge, isDirty: () => dirty, flushDraft: flush };
}

describe('window-close handshake (#93)', () => {
  beforeEach(() => {
    document.documentElement.dataset.theme = 'dark';
  });

  it('approves a clean close immediately with no prompt and no save', () => {
    const bridge = fakeBridge();
    const flush = vi.fn(async () => true);
    render(<Harness deps={depsOf(bridge, false, flush)} />);

    act(() => bridge.requestHostClose());

    expect(bridge.decisions).toEqual([true]);
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(flush).not.toHaveBeenCalled();
  });

  it('defers a dirty close and shows a save/cancel prompt', () => {
    const bridge = fakeBridge();
    render(<Harness deps={depsOf(bridge, true, async () => true)} />);

    act(() => bridge.requestHostClose());

    expect(bridge.decisions).toEqual([]);
    const dialog = screen.getByRole('alertdialog');
    expect(dialog).toHaveTextContent(/unsaved/i);
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Save & close' })).toBeInTheDocument();
  });

  it('cancel keeps the app open and does not approve the close', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const flush = vi.fn(async () => true);
    render(<Harness deps={depsOf(bridge, true, flush)} />);
    act(() => bridge.requestHostClose());

    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));

    expect(bridge.decisions).toEqual([false]);
    expect(flush).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
  });

  it('save flushes the draft before approving the close', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const order: string[] = [];
    const flush = vi.fn(async () => {
      order.push('flush');
      return true;
    });
    const tracked: CloseBridge = {
      onHostCloseRequest: bridge.onHostCloseRequest,
      async resolveClose(approved) {
        order.push(approved ? 'resolve' : 'cancel');
        await bridge.resolveClose(approved);
      },
    };
    render(<Harness deps={depsOf(tracked, true, flush)} />);
    act(() => bridge.requestHostClose());

    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Save & close' }));

    expect(order).toEqual(['flush', 'resolve']);
    expect(bridge.decisions).toEqual([true]);
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
  });

  it('a failed save keeps the dialog open with an error and never loses the edit', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const flush = vi.fn(async () => false);
    render(<Harness deps={depsOf(bridge, true, flush)} />);
    act(() => bridge.requestHostClose());

    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Save & close' }));

    expect(flush).toHaveBeenCalledTimes(1);
    expect(bridge.decisions).toEqual([]);
    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be saved/i);
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
  });

  it('beforeunload is only a complementary guard for dirty drafts', () => {
    const bridge = fakeBridge();
    render(<Harness deps={depsOf(bridge, true, async () => true)} />);

    const dirtyEvent = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(dirtyEvent);
    expect(dirtyEvent.defaultPrevented).toBe(true);
    expect(bridge.decisions).toEqual([]);
  });
});
