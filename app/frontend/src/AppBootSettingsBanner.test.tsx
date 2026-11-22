import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { mockController } from './api/mock';

describe('App boot settings-load error banner', () => {
  beforeEach(() => {
    window.localStorage.clear();
    mockController.reset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows a shell banner on boot-load failure and clears it after Settings retries successfully', async () => {
    const user = userEvent.setup();
    const realGetSettings = mockController.getSettings.bind(mockController);
    vi.spyOn(mockController, 'getSettings')
      .mockRejectedValueOnce(new Error('settings service unavailable'))
      .mockImplementation(() => realGetSettings());

    render(<App />);

    // The failure must be surfaced on the shell without opening Settings.
    const banner = await screen.findByRole('alert', { name: 'Settings load error' });
    expect(banner).toHaveTextContent(/fallback/i);
    expect(banner).toHaveTextContent(/settings service unavailable/i);

    // Opening Settings remounts the controller, which retries getSettings().
    // A successful retry must clear the boot banner.
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });

    await waitFor(() => {
      expect(screen.queryByRole('alert', { name: 'Settings load error' })).not.toBeInTheDocument();
    });
  });

  it('lets the user dismiss the boot-load error banner', async () => {
    const user = userEvent.setup();
    vi.spyOn(mockController, 'getSettings').mockRejectedValue(new Error('settings service unavailable'));

    render(<App />);

    const banner = await screen.findByRole('alert', { name: 'Settings load error' });
    expect(banner).toBeInTheDocument();

    await user.click(within(banner).getByRole('button', { name: 'Dismiss' }));

    await waitFor(() => {
      expect(screen.queryByRole('alert', { name: 'Settings load error' })).not.toBeInTheDocument();
    });
  });

  it('does not show the banner when boot getSettings rejects after settings were delivered', async () => {
    const user = userEvent.setup();
    const realGetSettings = mockController.getSettings.bind(mockController);
    let rejectBootLoad!: (reason?: unknown) => void;
    const bootLoad = new Promise<never>((_resolve, reject) => {
      rejectBootLoad = reject;
    });
    const getSettingsSpy = vi.spyOn(mockController, 'getSettings')
      .mockImplementationOnce(() => bootLoad)
      .mockImplementation(() => realGetSettings());

    render(<App />);

    // The boot request is still pending. Opening Settings remounts the
    // controller, whose retry succeeds and delivers real settings via onSaved.
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await waitFor(() => {
      expect(screen.queryByText('Loading settings…')).not.toBeInTheDocument();
    });

    // Now the original boot request rejects. Real settings have already been
    // delivered, so the late rejection must not resurrect the banner.
    await act(async () => {
      rejectBootLoad(new Error('settings service unavailable'));
    });

    expect(getSettingsSpy).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('alert', { name: 'Settings load error' })).not.toBeInTheDocument();
  });
});
