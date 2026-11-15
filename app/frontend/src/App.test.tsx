import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { AnalyzerView } from './components/AnalyzerView';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';
import { InfoTip } from './components/cleanShared';
import { mockController } from './api/mock';
import type { AnalyzerProgressEvent, ContractError, ScanItemDto, ScanProgressEvent, ScanResults, Settings, StartAnalyzerResult, SweeperClient, TargetScanResultDto } from './api/client';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('renders Scan & Clean shell and primary navigation', async () => {
    setup();
    expect(screen.getAllByText('Sweeper').length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: 'Scan & Clean' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('heading', { name: 'Scan & Clean' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
  });

  it('shows locked system-wide cleanup notice on Scan & Clean when non-elevated', async () => {
    setup();
    expect((await screen.findAllByLabelText('Standard (Non-Elevated)')).length).toBeGreaterThan(0);
    expect(screen.getAllByRole('button', { name: /Standard Mode/i }).length).toBeGreaterThan(0);
  });

  it('toggles Dark/Light theme and persists choice in localStorage', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    const appearance = screen.getByRole('group', { name: 'Appearance' });
    await user.click(within(appearance).getByRole('button', { name: 'Light' }));
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(window.localStorage.getItem('sweeper-theme')).toBe('light');
    await user.click(within(appearance).getByRole('button', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(window.localStorage.getItem('sweeper-theme')).toBe('dark');
  });

  it('InfoTip pins on first click, closes on second click, and exposes matching aria-describedby without dialog popup semantics', async () => {
    const user = userEvent.setup();
    render(
      <InfoTip label="About example">
        <p>Tooltip details</p>
      </InfoTip>,
    );
    const infoBtn = screen.getByRole('button', { name: 'About example' });
    expect(infoBtn).not.toHaveAttribute('aria-haspopup', 'dialog');

    await user.click(infoBtn);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Tooltip details');
    expect(infoBtn).toHaveAttribute('aria-describedby', tooltip.id);
    expect(infoBtn).toHaveAttribute('aria-expanded', 'true');

    await user.click(infoBtn);
    await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument());
    expect(infoBtn).not.toHaveAttribute('aria-describedby');

    await user.click(infoBtn);
    await screen.findByRole('tooltip');
    infoBtn.focus();
    fireEvent.keyDown(infoBtn, { key: 'Escape', code: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument());
  });

  it('runs full Scan -> Inspect -> Preview -> Clean summary -> Logs viewer flow', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    // auto-inspected item renders contextual explanation in Inspector (no Explain button/drawer)
    expect(await screen.findByText(/Temporary files Windows and apps can recreate automatically/i)).toBeInTheDocument();

    const eligibleCheckbox = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    if (!eligibleCheckbox.checked) await user.click(eligibleCheckbox);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await screen.findByText('Preview Breakdown');

    const cleanBtn = await screen.findByRole('button', { name: 'Clean Permanently' });
    expect(cleanBtn).toBeEnabled();
    await user.click(cleanBtn);

    await screen.findByRole('heading', { name: 'Clean Summary' });
    expect(screen.getByText('Bytes freed')).toBeInTheDocument();
    expect(screen.getByText('Deleted count')).toBeInTheDocument();
    expect(screen.getByText('Skipped count')).toBeInTheDocument();
    expect(screen.getByLabelText('Skipped reasons')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'View Operation Log' }));
    await screen.findByRole('heading', { name: 'Activity History' });
    expect(screen.getByRole('button', { name: 'Summary' })).toHaveAttribute('aria-pressed', 'true');
    await screen.findByLabelText('Structured log summary');
    await user.click(screen.getByRole('button', { name: 'Raw JSONL' }));
    await screen.findByLabelText('Log content');
    expect(screen.getByLabelText('Log content')).toHaveTextContent(/"record":"intent"/);
  });

  it('enforces typed confirmation for Custom Path/Unknown scan items and requires exact DELETE', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const expandCustomPathBtn = screen.queryByRole('button', { name: /Expand Custom Path/i });
    if (expandCustomPathBtn) await user.click(expandCustomPathBtn);

    const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb).toBeDisabled(); // gated by allow_delete_unknown

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    const allowUnknown = screen.getByLabelText('Allow deleting unknown folders (advanced)');
    await user.click(allowUnknown);
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const expandCustomPathBtn2 = screen.queryByRole('button', { name: /Expand Custom Path/i });
    if (expandCustomPathBtn2) await user.click(expandCustomPathBtn2);

    const unknownCb2 = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb2).toBeEnabled();
    await user.click(unknownCb2);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(previewBtn).toBeDisabled();

    await user.type(screen.getByLabelText('Typed confirmation'), 'delete');
    expect(previewBtn).toBeDisabled();
    await user.clear(screen.getByLabelText('Typed confirmation'));
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
    await user.click(previewBtn);
    await screen.findByText('Preview Breakdown');
  });

  it('shows non-elevated status, locks elevation targets, and reports UAC cancellation feedback safely', async () => {
    mockController.setRelaunchResult({
      error: { code: 'internal', details: { reason: 'Administrator launch was cancelled.' } },
    });
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    expect(screen.getAllByLabelText('Standard (Non-Elevated)').length).toBeGreaterThan(0);

    const elevationToggle = screen.getByLabelText(
      'Allow system-wide Windows caches (Admin)',
    ) as HTMLInputElement;
    expect(elevationToggle).toBeDisabled();
    expect(screen.getByText(/Deep Cleanup must be enabled first/i)).toBeInTheDocument();

    await user.click(screen.getAllByRole('button', { name: 'Run as administrator' })[0]);
    expect((await screen.findAllByRole('alert')).some((el) => /Administrator launch was cancelled/i.test(el.textContent || ''))).toBe(true);
  });

  it('allows enabling and persisting elevation targets when elevated', async () => {
    mockController.setElevationStatus({ isElevated: true });
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    expect(screen.getAllByLabelText('Administrator (Elevated)').length).toBeGreaterThan(0);

    await user.click(screen.getByLabelText('Enable deep cleanup categories'));
    const elevationToggle = screen.getByLabelText(
      'Allow system-wide Windows caches (Admin)',
    ) as HTMLInputElement;
    expect(elevationToggle).toBeEnabled();
    await user.click(elevationToggle);
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    const saved = await mockController.getSettings();
    expect(saved.enableElevationTargets).toBe(true);
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
        settings={{
          enableTier1: true,
          enableTier2: true,
          enableTier3: false,
          enableElevationTargets: false,
          minAgeHours: 24,
          allowDeleteUnknown: false,
          customPaths: [],
          staleProjectDays: 30,
        }}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onSaved={() => {}}
      />,
    );

    const recycleNote = await screen.findByLabelText('Recycle Bin included with Deep Cleanup');
    expect(recycleNote).toHaveTextContent('Controlled by Deep Cleanup');
    expect(screen.queryByLabelText(/Empty Windows Recycle Bin/i)).not.toBeInTheDocument();
    expect(screen.getByLabelText('Enable deep cleanup categories')).toBeInTheDocument();
    expect(screen.getByLabelText('Inactive project cache age')).toBeDisabled();

    const usePreset = screen.getByRole('button', { name: 'Use example custom path preset' });
    await user.click(usePreset);
    const customPathInput = screen.getByLabelText('Add Custom Path') as HTMLInputElement;
    expect(customPathInput.value).toMatch(/^C:\\Temp\\/);
    expect(customPathInput.value.toLowerCase()).not.toContain('\\users\\');
  });

  it('Analyzer gates Unknown selection behind allow_unknown and requires typed confirmation for Unknown nodes', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const unknownPath = 'C:\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache';
    let unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb) {
      // Root is expanded by default; if virtualization/state timing hides children in tests, re-toggle root.
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb) {
      // Fallback: locate the gated Unknown node row by its status text.
      const statuses = await screen.findAllByText(/Unknown gated by Settings/i);
      const row = statuses[0]?.closest('[role="treeitem"]') as HTMLElement | null;
      expect(row).not.toBeNull();
      unknownCb = within(row!).getByRole('checkbox') as HTMLInputElement;
    }
    expect(unknownCb).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByLabelText('Allow deleting unknown folders (advanced)'));
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    let unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb2) {
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb2) {
      // After allowing unknowns, select the first Unknown-selectable checkbox by finding a node whose status indicates typed confirmation required for Unknown/custom path.
      const candidates = screen.getAllByRole('checkbox', { name: /Select analyzer node /i }) as HTMLInputElement[];
      unknownCb2 = candidates.find((cb) => !cb.disabled) ?? null;
    }
    expect(unknownCb2).not.toBeNull();
    expect(unknownCb2!).toBeEnabled();
    await user.click(unknownCb2!);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(previewBtn).toBeDisabled();
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
  });

  it('handles expired/invalidated session errors cleanly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
    mockController.advanceTimeMs(15 * 60 * 1000 + 1000);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/expired|invalid/i);
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Inspector' })).not.toBeInTheDocument());
  });

  it('marks non-selectable analyzer nodes (.vhdx / Protected / Link Point) correctly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const all = screen.getAllByRole('checkbox') as HTMLInputElement[];
    const findBySuffix = (suffix: string) => all.find((n) => (n.getAttribute('aria-label') ?? '').includes(suffix));
    const vhdx = findBySuffix('dev-machine.vhdx');
    expect(vhdx).toBeTruthy();
    expect(vhdx!).toBeDisabled();
    expect(vhdx!.closest('[role="treeitem"]')).toHaveTextContent(/\.vhdx/i);

    const gitNode = findBySuffix('\\Dev\\repo\\.git');
    expect(gitNode).toBeTruthy();
    expect(gitNode!).toBeDisabled();
    expect(gitNode!.closest('[role="treeitem"]')).toHaveTextContent(/Protected/i);

    const linkNode = findBySuffix('\\Users\\Alice\\OneDriveLink');
    expect(linkNode).toBeTruthy();
    expect(linkNode!).toBeDisabled();
    expect(linkNode!.closest('[role="treeitem"]')).toHaveTextContent(/Link Point/i);
  });

  it('supports scan bulk selection, target-group selection, and status filters', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const selectAllSafe = screen.getByRole('button', { name: 'Select Safe' });
    expect(selectAllSafe).toBeEnabled();
    await user.click(selectAllSafe);

    const safeEligible = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    expect(safeEligible).toBeChecked();
    expect(screen.getAllByRole('button', { name: 'Clear' }).length).toBeGreaterThan(0);

    expect(screen.getAllByLabelText('Status Skipped').length).toBeGreaterThan(0);
    await user.click(screen.getByRole('button', { name: /^Eligible \(/ }));
    await waitFor(() => expect(screen.queryAllByLabelText('Status Skipped')).toHaveLength(0));

    await user.click(screen.getAllByRole('button', { name: 'Clear' })[0]);
    await waitFor(() => expect(safeEligible.checked).toBe(false));

    await user.click(screen.getByRole('button', { name: /^All \(/ }));
    const userTempGroup = screen.getByLabelText('Select all eligible items in User Temp') as HTMLInputElement;
    await user.click(userTempGroup);
    expect(safeEligible).toBeChecked();
  });

  it('shows Analyzer sticky action bar Review & Clean with disabled/enabled states', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' }) as HTMLButtonElement;
    expect(previewBtn).toBeDisabled();

    const safeNode = await screen.findByLabelText(/Select analyzer node .*AppData\\Local\\Temp$/);
    await user.click(safeNode);
    expect(previewBtn).toBeEnabled();
    expect(previewBtn.className).toMatch(/primary/);
  });

  it('renders incremental scan progress updates (Bug 2)', async () => {
    const { user } = userEvent.setup ? { user: userEvent.setup() } : { user: userEvent };
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const scanResults: ScanResults = {
      sessionId: 's1',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [],
    };
    const client = {
      startScan: async (_tiers: unknown, onProgress?: (e: ScanProgressEvent) => void) => {
        onProgress?.({ kind: 'started', session_id: 's1' });
        onProgress?.({
          kind: 'progress',
          session_id: 's1',
          completed_targets: 1,
          total_targets: 3,
          current_target_id: 't1',
          current_target_name: 'User Temp',
          scanned_items: '12',
          scanned_bytes: '1048576',
        });
        await gate;
        onProgress?.({ kind: 'completed', session_id: 's1' });
        return { sessionId: 's1' };
      },
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={null} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    const notice = await screen.findByRole('status');
    expect(notice).toHaveClass('scan-progress-notice');
    expect(within(notice).getByText(/1 \/ 3 Targets/)).toBeInTheDocument();
    expect(within(notice).getAllByText(/User Temp/).length).toBeGreaterThan(0);
    expect(within(notice).getByText(/12 items ·/)).toBeInTheDocument();
    expect(notice.querySelector('.scan-progress-meter')).not.toBeNull();
    release();
    await waitFor(() => expect(screen.queryByText(/1 \/ 3 Targets/)).not.toBeInTheDocument());
  });

  it('renders analyzer long-path progress in bounded overflow-safe container (Bug 3a)', async () => {
    const { user } = userEvent.setup ? { user: userEvent.setup() } : { user: userEvent };
    const longPath = 'C:\\Users\\Alice\\AppData\\Local\\Temp\\sweeper-sandbox-FwSDTi\\target\\d0000\\d0001\\d0002\\d0003\\very-long-unbroken-directory-name-without-spaces-for-overflow-testing';
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const minimalRoot = {
      id: 'root',
      name: 'C:',
      path: 'C:\\',
      isDir: true,
      sizeBytes: '0',
      itemCount: '0',
      risk: 'Safe' as const,
      description: 'root',
      reason: null,
      tier: null,
      targetId: null,
      isLinkPoint: false,
      isCloudPlaceholder: false,
      displayOnly: false,
      selectable: false,
      children: [],
    };
    const client = {
      startAnalyzer: async (_root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
        onProgress?.({ sessionId: 'a1', scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: 'C:\\' });
        onProgress?.({ sessionId: 'a1', scannedDirs: '42', scannedFiles: '7', scannedBytes: '4096', currentPath: longPath });
        await gate;
        return {
          sessionId: 'a1',
          rootPath: 'C:\\',
          totalBytes: '0',
          totalItems: '0',
          scannedDirs: '42',
          scannedFiles: '7',
          cancelled: false,
          warnings: [],
          root: minimalRoot,
        };
      },
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    const notice = await screen.findByRole('status');
    expect(notice).toHaveClass('analyzer-progress-notice');
    expect(notice).toHaveClass('scanning-pulse');
    const pathEl = notice.querySelector('.analyzer-progress-path');
    expect(pathEl).not.toBeNull();
    expect(pathEl).toHaveAttribute('title', longPath);
    expect(pathEl?.textContent).toContain('sweeper-sandbox-FwSDTi');
    release();
    await waitFor(() => expect(screen.queryByText(/Analyzing…/)).not.toBeInTheDocument());
  });

  it('preserves Scan & Clean and Analyzer state across navigation (Bug 4)', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    expect(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
    expect(await screen.findByRole('heading', { name: 'Inspector' })).toBeInTheDocument();
    expect(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    expect(await screen.findByRole('heading', { name: 'Analyzer Tree' })).toBeInTheDocument();
  });

  it('ScanAndPreviewView discards stale previewClean response when selection changes while pending', async () => {
    const user = userEvent.setup();
    let resolvePreview!: (value: any) => void;
    const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
    const scanResults: ScanResults = {
      sessionId: 'scan-1',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [
        {
          targetId: 'user-temp',
          targetName: 'User Temp',
          tier: 'Tier1',
          risk: 'Safe',
          statusReason: null,
          totalBytes: '200',
          eligibleBytes: '200',
          itemCount: '2',
          scanned: true,
          resolvedPaths: ['C:\\Users\\Alice\\AppData\\Local\\Temp'],
          items: [
            { id: 'i1', targetId: 'user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
            { id: 'i2', targetId: 'user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          ],
        } as any,
      ],
    };
    let previewCalls = 0;
    const client = {
      startScan: async () => ({ sessionId: 'scan-1' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: () => { previewCalls += 1; return previewPromise; },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const cb1 = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    const cb2 = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\b.tmp') as HTMLInputElement;
    if (!cb1.checked) await user.click(cb1);
    if (!cb2.checked) await user.click(cb2);

    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(previewCalls).toBe(1));

    await user.click(cb2); // change selection while preview pending
    resolvePreview({
      sessionId: 'scan-1',
      previewToken: 'tok-stale',
      issuedAtUnixSecs: '1710000000',
      expiresAtUnixSecs: '1710000600',
      totalBytes: '200',
      totalItems: '2',
      requiresConfirmation: false,
      requiresElevation: false,
      perTarget: [],
    });

    await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Clean Permanently' })).not.toBeInTheDocument();
    expect(screen.queryByText('Preview Breakdown')).not.toBeInTheDocument();
  });

  it('AnalyzerView discards stale previewClean response when selection changes while pending', async () => {
    const user = userEvent.setup();
    let resolvePreview!: (value: any) => void;
    const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-1',
      rootPath: 'C:\\',
      totalBytes: '200',
      totalItems: '2',
      scannedDirs: '1',
      scannedFiles: '2',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root',
        name: 'C:',
        path: 'C:\\',
        isDir: true,
        sizeBytes: '200',
        itemCount: '2',
        risk: 'Safe',
        description: 'root',
        reason: null,
        tier: null,
        targetId: null,
        isLinkPoint: false,
        isCloudPlaceholder: false,
        displayOnly: false,
        selectable: false,
        children: [
          {
            id: 'n1',
            name: 'SafeA',
            path: 'C:\\SafeA',
            isDir: true,
            sizeBytes: '100',
            itemCount: '1',
            risk: 'Safe',
            description: 'safe',
            reason: null,
            tier: 'Tier1',
            targetId: 't1',
            isLinkPoint: false,
            isCloudPlaceholder: false,
            displayOnly: false,
            selectable: true,
            children: [],
          },
          {
            id: 'n2',
            name: 'SafeB',
            path: 'C:\\SafeB',
            isDir: true,
            sizeBytes: '100',
            itemCount: '1',
            risk: 'Safe',
            description: 'safe',
            reason: null,
            tier: 'Tier1',
            targetId: 't1',
            isLinkPoint: false,
            isCloudPlaceholder: false,
            displayOnly: false,
            selectable: true,
            children: [],
          },
        ],
      },
    };
    let previewCalls = 0;
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: () => { previewCalls += 1; return previewPromise; },
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const cb1 = await screen.findByLabelText('Select analyzer node C:\\SafeA') as HTMLInputElement;
    const cb2 = await screen.findByLabelText('Select analyzer node C:\\SafeB') as HTMLInputElement;
    await user.click(cb1);
    await user.click(cb2);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await waitFor(() => expect(previewCalls).toBe(1));

    await user.click(cb2); // change selection while preview pending
    resolvePreview({
      sessionId: 'an-1',
      previewToken: 'tok-stale-an',
      issuedAtUnixSecs: '1710000000',
      expiresAtUnixSecs: '1710000600',
      totalBytes: '200',
      totalItems: '2',
      requiresConfirmation: false,
      requiresElevation: false,
      perTarget: [],
    });

    await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Clean Permanently' })).not.toBeInTheDocument();
    expect(screen.queryByText('Preview Breakdown')).not.toBeInTheDocument();
  });

  it('AnalyzerView escapes VHDX PowerShell path as single-quoted literal in render and copy', async () => {
    const user = userEvent.setup();
    let copied = '';
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (text: string) => { copied = text; } },
    });
    try {
      const maliciousPath = "C:\\VMs\\evil'$(Start-Process calc).vhdx";
      const analyzerResult: StartAnalyzerResult = {
        sessionId: 'an-vhdx',
        rootPath: 'C:\\',
        totalBytes: '10',
        totalItems: '1',
        scannedDirs: '1',
        scannedFiles: '1',
        cancelled: false,
        warnings: [],
        root: {
          id: 'root',
          name: 'C:',
          path: 'C:\\',
          isDir: true,
          sizeBytes: '10',
          itemCount: '1',
          risk: 'Safe',
          description: 'root',
          reason: null,
          tier: null,
          targetId: null,
          isLinkPoint: false,
          isCloudPlaceholder: false,
          displayOnly: false,
          selectable: false,
          children: [
            {
              id: 'v1',
              name: "evil'$(Start-Process calc).vhdx",
              path: maliciousPath,
              isDir: false,
              sizeBytes: '10',
              itemCount: '1',
              risk: 'Caution',
              description: 'vhdx',
              reason: null,
              tier: null,
              targetId: null,
              isLinkPoint: false,
              isCloudPlaceholder: false,
              displayOnly: true,
              selectable: false,
              children: [],
            },
          ],
        },
      };
      const client = {
        startAnalyzer: async () => analyzerResult,
        explainItem: async (itemId: string) => ({
          itemId,
          title: itemId === 'v1' ? "evil'$(Start-Process calc).vhdx" : 'root',
          description: 'desc',
          risk: 'Caution' as const,
        }),
      } as unknown as SweeperClient;

      render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
      await user.click(screen.getByRole('button', { name: 'Analyze' }));
      await screen.findByRole('heading', { name: 'Analyzer Tree' });
      const vhdxRow = (await screen.findAllByText(/\.vhdx$/i))[0].closest('[role="treeitem"]') as HTMLElement;
      await user.click(vhdxRow);

      const expectedEscapedPath = "'C:\\VMs\\evil''$(Start-Process calc).vhdx'";
      const expectedCmd = `wsl --shutdown\nOptimize-VHD -Path ${expectedEscapedPath} -Mode Full`;
      expect(await screen.findByText(/Optimize-VHD -Path/)).toBeInTheDocument();
      const cmdNode = screen.getByText(/Optimize-VHD -Path/).closest('code');
      expect(cmdNode?.textContent).toBe(expectedCmd);
      expect(cmdNode?.textContent).not.toContain(`"${maliciousPath}"`);

      await user.click(screen.getByRole('button', { name: 'Copy Command' }));
      await waitFor(() => expect(copied).toBe(expectedCmd));
    } finally {
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: originalClipboard });
    }
  });

  it('ScanAndPreviewView keeps eligible Caution/Unknown group statuses and uses dynamic min age labels', async () => {
    const user = userEvent.setup();
    const cautionTarget: TargetScanResultDto = {
      targetId: 't-caution',
      targetName: 'Vendor Logs',
      tier: 'Tier2',
      risk: 'Caution',
      resolvedPaths: ['C:\\Temp\\VendorLogs'],
      scanned: true,
      statusReason: null,
      totalBytes: '300',
      eligibleBytes: '200',
      fileCount: '2',
      eligibleCount: '1',
      skippedCount: '1',
      items: [
        { id: 'c-old', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\old.log', size: '200', mtimeUnixSecs: '1700000000', ageEligible: true },
        { id: 'c-new', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\new.log', size: '100', mtimeUnixSecs: '1899999999', ageEligible: false },
      ],
    };
    const unknownTarget: TargetScanResultDto = {
      targetId: 'custom-path-1',
      targetName: 'Custom Path (Unknown)',
      tier: 'Tier2',
      risk: 'Unknown',
      resolvedPaths: ['C:\\Temp\\CustomCleanup'],
      scanned: true,
      statusReason: null,
      totalBytes: '50',
      eligibleBytes: '50',
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: [
        { id: 'u-old', targetId: 'custom-path-1', path: 'C:\\Temp\\CustomCleanup\\old.bin', size: '50', mtimeUnixSecs: '1700000000', ageEligible: true },
      ],
    };
    const scanResults: ScanResults = {
      sessionId: 'scan-risk',
      startedUnixSecs: '1710000000',
      minAgeHours: 48,
      targets: [cautionTarget, unknownTarget],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-risk' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(
      <ScanAndPreviewView
        client={client}
        settings={{ allowDeleteUnknown: true, minAgeHours: 24 } as Settings}
        settingsVersion={0}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onOpenLog={() => {}}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const cautionGroup = screen.getByLabelText('Inspect Vendor Logs').closest('article') as HTMLElement;
    const cautionGroupHeader = within(cautionGroup).getByLabelText('Inspect Vendor Logs');
    expect(within(cautionGroupHeader).getByLabelText('Status Caution')).toHaveTextContent(/1 Caution|Caution/i);
    expect(within(cautionGroupHeader).queryByLabelText('Status Safe')).not.toBeInTheDocument();

    const unknownGroup = screen.getByLabelText('Inspect Custom Path (Unknown)').closest('article') as HTMLElement;
    const unknownGroupHeader = within(unknownGroup).getByLabelText('Inspect Custom Path (Unknown)');
    expect(within(unknownGroupHeader).getByLabelText('Status Unknown')).toHaveTextContent(/Unknown/i);
    expect(within(unknownGroupHeader).queryByLabelText('Status Safe')).not.toBeInTheDocument();

    expect(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\new.log').closest('[role="button"]')).toHaveTextContent('< 48h');
    await user.click(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\old.log'));
    const over48 = await screen.findAllByText(/> 48h/);
    expect(over48.length).toBeGreaterThan(0);
    expect(screen.queryByText(/> 24h/)).not.toBeInTheDocument();
    expect(screen.queryByText(/< 24h/)).not.toBeInTheDocument();
  });

  it('ScanAndPreviewView ignores stale explain responses and invalidates in-flight explain on scan reset', async () => {
    const user = userEvent.setup();
    const deferredByItem = new Map<string, Array<{ resolve: (v: any) => void; promise: Promise<any> }>>();
    let scanCount = 0;
    let allowAutoInspect = true;
    const scanResults1: ScanResults = {
      sessionId: 'scan-1',
      startedUnixSecs: '1710000000',
      minAgeHours: 48,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '300', eligibleBytes: '300', fileCount: '3', eligibleCount: '3', skippedCount: '0',
        items: [
          { id: 'a', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'b', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'c', targetId: 't-safe', path: 'C:\\Temp\\c.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
        ],
      }],
    };
    const scanResults2: ScanResults = { ...scanResults1, sessionId: 'scan-2' };
    const deferExplain = (itemId: string) => {
      let resolve!: (v: any) => void;
      const promise = new Promise<any>((r) => { resolve = r; });
      const list = deferredByItem.get(itemId) ?? [];
      list.push({ resolve, promise });
      deferredByItem.set(itemId, list);
      return promise;
    };
    const client = {
      startScan: async () => ({ sessionId: scanCount === 0 ? 'scan-1' : 'scan-2' }),
      getScanResults: async () => (scanCount++ === 0 ? scanResults1 : scanResults2),
      explainItem: (itemId: string) => {
        if (allowAutoInspect) {
          allowAutoInspect = false;
          return Promise.resolve({ itemId, title: `${itemId} initial`, description: 'init', risk: 'Safe' as const });
        }
        return deferExplain(itemId);
      },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    // Initial auto-inspect focuses item a and shows its details.
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('a.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText('Select C:\\Temp\\a.tmp')); // focus a (older pending explain)
    await user.click(screen.getByLabelText('Select C:\\Temp\\b.tmp')); // focus b (latest pending explain)
    const aDeferred = deferredByItem.get('a')?.[0];
    const bDeferred = deferredByItem.get('b')?.[0];
    expect(aDeferred).toBeTruthy();
    expect(bDeferred).toBeTruthy();

    await act(async () => {
      bDeferred!.resolve({ itemId: 'b', title: 'Item B latest', description: 'desc B', risk: 'Safe' });
      await bDeferred!.promise;
    });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
    });

    await act(async () => {
      aDeferred!.resolve({ itemId: 'a', title: 'Item A stale', description: 'desc A', risk: 'Safe' });
      await aDeferred!.promise;
    });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
      expect(within(inspector).queryByText('Select an item to see its purpose, risk, full path, and eligibility.')).not.toBeInTheDocument();
    });

    // Start a new pending explain for c, then rescan to invalidate it.
    allowAutoInspect = false; // next explain call after reset should be deferred (rescan auto-focus)
    await user.click(screen.getByLabelText('Select C:\\Temp\\c.tmp'));
    const cDeferred = deferredByItem.get('c')?.[0];
    expect(cDeferred).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await act(async () => {
      cDeferred!.resolve({ itemId: 'c', title: 'Item C stale reset', description: 'stale', risk: 'Safe' });
      await cDeferred!.promise;
    });
    // Rescan clears/invalidates in-flight explain; stale c response must not repopulate item c context.
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).queryByText('c.tmp', { selector: '.inspector-headline-name' })).not.toBeInTheDocument();
    });
  });

  it('ScanAndPreviewView does not hijack keydown from nested row controls and shows no hard-coded 24h at min age 0', async () => {
    const user = userEvent.setup();
    const scanResults: ScanResults = {
      sessionId: 'scan-keys',
      startedUnixSecs: '1710000000',
      minAgeHours: 0,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0',
        items: [{ id: 'k1', targetId: 't-safe', path: 'C:\\Temp\\k.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }],
      }],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-keys' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false, minAgeHours: 0 } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    await user.click(screen.getByRole('button', { name: 'Clear' }));
    const cb = screen.getByLabelText('Select C:\\Temp\\k.tmp') as HTMLInputElement;
    cb.focus();
    expect(cb).not.toBeChecked();
    fireEvent.keyDown(cb, { key: 'Enter', code: 'Enter', bubbles: true });
    expect(cb).not.toBeChecked();
    fireEvent.click(cb); // realistic toggle via control interaction
    await waitFor(() => expect(cb).toBeChecked());
    fireEvent.keyDown(cb, { key: ' ', code: 'Space', bubbles: true });
    expect(cb).toBeChecked();
    expect(screen.queryByText(/< 24h/)).not.toBeInTheDocument();
    expect(screen.queryByText(/24h/)).not.toBeInTheDocument();
  });

  it('AnalyzerView exposes Use Preset with a neutral root preset', async () => {
    const user = userEvent.setup();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-preset',
      rootPath: 'C:\\Users',
      totalBytes: '0',
      totalItems: '0',
      scannedDirs: '0',
      scannedFiles: '0',
      cancelled: false,
      warnings: [],
      root: { id: 'root', name: 'C:\\Users', path: 'C:\\Users', isDir: true, sizeBytes: '0', itemCount: '0', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [] },
    };
    const startAnalyzer = vi.fn(async (path: string | null) => ({ ...analyzerResult, rootPath: path ?? analyzerResult.rootPath, root: { ...analyzerResult.root, name: path ?? analyzerResult.root.name, path: path ?? analyzerResult.root.path } }));
    const client = {
      startAnalyzer,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as Settings} onOpenLog={() => {}} />);
    const usePreset = screen.getByRole('button', { name: 'Use preset root path' });
    await user.click(usePreset);
    const rootInput = screen.getByLabelText('Analyzer root path') as HTMLInputElement;
    expect(['C:\\', 'C:\\Users', 'C:\\Windows\\Temp']).toContain(rootInput.value);
    expect(rootInput.value.toLowerCase()).not.toMatch(/\\users\\[^\\]/);
  });

  it('AnalyzerView keeps selectable Unknown under Unknown and Select Safe dedupes non-overlapping safe roots', async () => {
    const user = userEvent.setup();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-dedupe',
      rootPath: 'C:\\',
      totalBytes: '500',
      totalItems: '5',
      scannedDirs: '3',
      scannedFiles: '4',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '500', itemCount: '5', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children: [
          {
            id: 'safe-parent', name: 'Temp', path: 'C:\\Temp', isDir: true, sizeBytes: '300', itemCount: '3', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
            children: [
              { id: 'safe-child-a', name: 'a.tmp', path: 'C:\\Temp\\a.tmp', isDir: false, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
              { id: 'safe-child-b', name: 'b.tmp', path: 'C:\\Temp\\b.tmp', isDir: false, sizeBytes: '200', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
            ],
          },
          {
            id: 'unknown-sel', name: 'Mystery', path: 'C:\\Mystery', isDir: true, sizeBytes: '50', itemCount: '1', risk: 'Unknown', description: 'unknown', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
            children: [
              { id: 'unknown-child', name: 'x.bin', path: 'C:\\Mystery\\x.bin', isDir: false, sizeBytes: '50', itemCount: '1', risk: 'Unknown', description: 'unknown', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
            ],
          },
          { id: 'caution-node', name: 'Logs', path: 'C:\\Logs', isDir: false, sizeBytes: '150', itemCount: '1', risk: 'Caution', description: 'caution', reason: null, tier: 'Tier2', targetId: 't-logs', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
        ],
      },
    };
    const previewClean = vi.fn(async (_sid: string, itemIds: string[]) => ({
      previewToken: 'tok',
      breakdown: [{ targetId: 't-temp', targetName: 'Temp', selectedCount: String(itemIds.length), selectedBytes: String(itemIds.includes('safe-parent') ? 300 : 0) }],
      totalSelectedCount: String(itemIds.length),
      totalSelectedBytes: String(itemIds.includes('safe-parent') ? 300 : 0),
    }));
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: itemId.includes('unknown') ? 'Unknown' as const : itemId.includes('caution') ? 'Caution' as const : 'Safe' as const }),
      previewClean,
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: true } as Settings} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    expect(screen.getByRole('button', { name: 'Unknown (1)' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Safe (1)' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Unknown (1)' }));
    expect(screen.getByLabelText('Select analyzer node C:\\Mystery')).toBeInTheDocument();
    expect(screen.queryByLabelText('Select analyzer node C:\\Temp')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /^All \(/ }));

    await user.click(screen.getByRole('button', { name: 'Select Safe' }));
    expect(screen.getByLabelText('Select analyzer node C:\\Temp')).toBeChecked();
    const tempRow = screen.getByLabelText('Select analyzer node C:\\Temp').closest('[role="treeitem"]') as HTMLElement;
    await user.click(within(tempRow).getByRole('button', { name: /Expand Temp/i }));
    expect(await screen.findByLabelText('Select analyzer node C:\\Temp\\a.tmp')).not.toBeChecked();
    expect(screen.getByLabelText('Select analyzer node C:\\Temp\\b.tmp')).not.toBeChecked();
    expect(screen.getByText(/1 item selected/i)).toBeInTheDocument();
    const reviewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(reviewBtn).toHaveTextContent('300 B');

    await user.click(reviewBtn);
    await waitFor(() => expect(previewClean).toHaveBeenCalled());
    expect(previewClean.mock.calls[0][1]).toEqual(['safe-parent']);
  });

  it('AnalyzerView blocks selection/drill while cleaning, ignores bubbled row keydown, and keeps treemap within viewBox for 100 equal children', async () => {
    const user = userEvent.setup();
    let resolveClean!: (v: any) => void;
    const cleanPromise = new Promise((r) => { resolveClean = r; });
    const children = Array.from({ length: 100 }, (_, i) => ({
      id: `n-${i}`, name: `f-${i}.tmp`, path: `C:\\Bulk\\f-${i}.tmp`, isDir: false, sizeBytes: '10', itemCount: '1', risk: 'Safe' as const,
      description: 'safe', reason: null, tier: 'Tier1' as const, targetId: 't', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [],
    }));
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-ui',
      rootPath: 'C:\\',
      totalBytes: '1000',
      totalItems: '100',
      scannedDirs: '1',
      scannedFiles: '100',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '1000', itemCount: '100', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children,
      },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: async () => ({
        previewToken: 'tok-ui',
        breakdown: [{ targetId: 't', targetName: 'Bulk', selectedCount: '1', selectedBytes: '10' }],
        totalSelectedCount: '1',
        totalSelectedBytes: '10',
      }),
      executeClean: () => cleanPromise,
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as Settings} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const rects = Array.from(document.querySelectorAll('svg.alloc-bar rect.alloc-segment'));
    expect(rects.length).toBeGreaterThan(0);
    for (const r of rects) {
      const x = Number(r.getAttribute('x') || '0');
      const w = Number(r.getAttribute('width') || '0');
      expect(x).toBeGreaterThanOrEqual(0);
      expect(w).toBeGreaterThanOrEqual(0);
      expect(x + w).toBeLessThanOrEqual(1000.001);
    }

    const firstCheckbox = screen.getByLabelText('Select analyzer node C:\\Bulk\\f-0.tmp') as HTMLInputElement;
    await user.click(firstCheckbox);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await screen.findByRole('button', { name: 'Clean Permanently' });
    await user.click(screen.getByRole('button', { name: 'Clean Permanently' }));
    expect(screen.getByText(/Cleaning in progress/i)).toBeInTheDocument();

    const row = firstCheckbox.closest('[role="treeitem"]') as HTMLElement;
    await user.click(row);
    expect(firstCheckbox).toBeChecked();
    firstCheckbox.focus();
    fireEvent.keyDown(firstCheckbox, { key: 'Enter', code: 'Enter', bubbles: true });
    expect(firstCheckbox).toBeChecked();
    const drillBtn = within(row).queryByRole('button', { name: /Drill into/i });
    if (drillBtn) expect(drillBtn).toBeDisabled();

    await act(async () => { resolveClean({ bytesFreed: '10', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
  });

  it('AnalyzerView ignores late explainItem response after focusing another node', async () => {
    const user = userEvent.setup();
    const deferred = new Map<string, { resolve: (v: any) => void }>();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-late',
      rootPath: 'C:\\',
      totalBytes: '200',
      totalItems: '2',
      scannedDirs: '1',
      scannedFiles: '2',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '200', itemCount: '2', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children: [
          { id: 'a', name: 'SafeA', path: 'C:\\SafeA', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
          { id: 'b', name: 'SafeB', path: 'C:\\SafeB', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
        ],
      },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: (itemId: string) => new Promise<any>((resolve) => { deferred.set(itemId, { resolve }); }),
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const cbA = await screen.findByLabelText('Select analyzer node C:\\SafeA');
    await user.click(cbA);
    const cbB = await screen.findByLabelText('Select analyzer node C:\\SafeB');
    await user.click(cbB);

    deferred.get('b')?.resolve({ itemId: 'b', title: 'SafeB', description: 'descB', risk: 'Safe' });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Analyzer inspector');
      expect(within(inspector).getAllByText('SafeB').length).toBeGreaterThan(0);
    });

    deferred.get('a')?.resolve({ itemId: 'a', title: 'SafeA', description: 'descA', risk: 'Safe' });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Analyzer inspector');
      expect(within(inspector).getAllByText('SafeB').length).toBeGreaterThan(0);
      expect(within(inspector).queryByText('SafeA')).not.toBeInTheDocument();
    });
  });
});