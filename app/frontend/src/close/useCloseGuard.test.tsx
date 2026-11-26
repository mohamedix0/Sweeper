import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CloseDraftDialog } from '../components/CloseDraftDialog';
import type { CloseBridge } from './closeBridge';
import { fakeCloseBridge as fakeBridge } from './fakeCloseBridge';
import { useCloseGuard, type CloseGuardDeps } from './useCloseGuard';

function Harness({ deps }: { deps: CloseGuardDeps }) {
  const guard = useCloseGuard(deps);
  return (
    <div>
      <output data-testid="prompt">{String(guard.promptOpen)}</output>
      <output data-testid="close-pending">{String(guard.closePending)}</output>
      {guard.error && !guard.promptOpen && (
        <p className="notice error close-error-notice" role="alert">
          {guard.error}
          {guard.canRetry && <button type="button" onClick={() => void guard.retryDelivery()}>Retry</button>}
        </p>
      )}
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

function failFirstDelivery(bridge: CloseBridge): CloseBridge {
  let attempts = 0;
  return {
    ...bridge,
    async resolveClose(approved, requestId) {
      attempts += 1;
      await bridge.resolveClose(approved, requestId);
      if (attempts === 1) throw new Error('ipc down');
    },
  };
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
      async resolveClose(approved, requestId) {
        order.push(approved ? 'resolve' : 'cancel');
        await bridge.resolveClose(approved, requestId);
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

  it('locks controls from a clean approval through exit', async () => {
    const bridge = fakeBridge();
    render(<Harness deps={depsOf(bridge, false, async () => true)} />);
    expect(screen.getByTestId('close-pending')).toHaveTextContent('false');

    act(() => bridge.requestHostClose());

    expect(bridge.decisions).toEqual([true]);
    await waitFor(() => expect(screen.getByTestId('close-pending')).toHaveTextContent('true'));
  });

  it('locks controls while the approval is still in flight', async () => {
    const bridge = fakeBridge();
    let finishResolve!: () => void;
    const tracked: CloseBridge = {
      ...bridge,
      resolveClose: (approved, requestId) =>
        new Promise<void>((resolve) => {
          finishResolve = () => {
            void bridge.resolveClose(approved, requestId);
            resolve();
          };
        }),
    };
    render(<Harness deps={depsOf(tracked, false, async () => true)} />);

    act(() => bridge.requestHostClose());
    // Rust blocks its answer until in-flight Cleans settle; controls must be
    // locked from here, not only once the IPC promise resolves (#93).
    await waitFor(() => expect(screen.getByTestId('close-pending')).toHaveTextContent('true'));
    expect(bridge.decisions).toEqual([]);

    finishResolve();
    await waitFor(() => expect(bridge.decisions).toEqual([true]));
  });

  it('cancel is inert while saving so a stale cancel cannot answer a newer close', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    let finishFlush!: (value: boolean) => void;
    const flush = vi.fn(() => new Promise<boolean>((resolve) => { finishFlush = resolve; }));
    render(<Harness deps={depsOf(bridge, true, flush)} />);
    act(() => bridge.requestHostClose());
    const dialog = screen.getByRole('alertdialog');
    await user.click(within(dialog).getByRole('button', { name: 'Save & close' }));

    // Saving is in flight: Escape and Cancel must not send a decision that
    // could later answer a newer close request (#93).
    await user.keyboard('{Escape}');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(bridge.decisions).toEqual([]);

    finishFlush(true);
    await waitFor(() => expect(bridge.decisions).toEqual([true]));
  });

  it('a stale cancel completion cannot clear or hide a newer close request', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    let finishR1!: () => void;
    const resolveR1 = vi.fn((_approved: boolean, _requestId: string) =>
      new Promise<void>((resolve) => { finishR1 = resolve; }),
    );
    const tracked: CloseBridge = {
      ...bridge,
      resolveClose: (approved, requestId) =>
        requestId === 'R1' ? resolveR1(approved, requestId) : bridge.resolveClose(approved, requestId),
    };
    render(<Harness deps={depsOf(tracked, true, async () => true)} />);
    act(() => bridge.requestHostClose('R1'));
    const dialog = screen.getByRole('alertdialog');

    // Cancel R1, but keep its IPC answer in flight.
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(resolveR1).toHaveBeenCalledTimes(1);

    // A newer close R2 supersedes R1 before R1's decision settles.
    act(() => bridge.requestHostClose('R2'));

    // R1's stale completion must leave R2's id and prompt intact.
    finishR1();
    await waitFor(() => expect(screen.getByTestId('prompt')).toHaveTextContent('true'));

    // The still-open prompt answers R2, not the superseded R1.
    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    expect(bridge.decisionIds).toEqual(['R2']);
  });

  it('a failed decision delivery surfaces an error and keeps the prompt open', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const failing: CloseBridge = {
      ...bridge,
      resolveClose: vi.fn(async () => { throw new Error('ipc down'); }),
    };
    render(<Harness deps={depsOf(failing, true, async () => true)} />);
    act(() => bridge.requestHostClose());
    const dialog = screen.getByRole('alertdialog');
    await user.click(within(dialog).getByRole('button', { name: 'Save & close' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be delivered/i);
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
    expect(screen.getByTestId('close-pending')).toHaveTextContent('false');
  });

  it('Retry re-delivers the same request id and decision and clears the close error notice', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const tracked = failFirstDelivery(bridge);
    render(<Harness deps={depsOf(tracked, false, async () => true)} />);
    act(() => bridge.requestHostClose('R1'));

    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be delivered/i);
    await user.click(screen.getByRole('button', { name: 'Retry' }));

    await waitFor(() => expect(bridge.decisions).toEqual([true, true]));
    expect(bridge.decisionIds).toEqual(['R1', 'R1']);
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });

  it('a clean close with a failed delivery shows an error without reopening the dialog', async () => {
    const bridge = fakeBridge();
    const failing: CloseBridge = {
      ...bridge,
      resolveClose: vi.fn(async () => { throw new Error('ipc down'); }),
    };
    render(<Harness deps={depsOf(failing, false, async () => true)} />);
    act(() => bridge.requestHostClose());

    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be delivered/i);
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.getByTestId('prompt')).toHaveTextContent('false');
  });

  it('Retry reopens the decision when the draft became dirty after a failed clean close', async () => {
    const user = userEvent.setup();
    const bridge = fakeBridge();
    const failing: CloseBridge = { ...bridge, resolveClose: vi.fn(async () => { throw new Error('ipc down'); }) };
    let dirty = false;
    render(<Harness deps={{ bridge: failing, isDirty: () => dirty, flushDraft: async () => true }} />);
    act(() => bridge.requestHostClose('R3'));

    expect(await screen.findByRole('alert')).toHaveTextContent(/could not be delivered/i);
    dirty = true;
    await user.click(screen.getByRole('button', { name: 'Retry' }));

    expect(await screen.findByRole('alertdialog')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(failing.resolveClose).toHaveBeenCalledTimes(1);
  });
});
