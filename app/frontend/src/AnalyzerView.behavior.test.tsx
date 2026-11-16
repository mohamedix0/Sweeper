import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
import type { AnalyzerProgressEvent, Settings, StartAnalyzerResult, SweeperClient } from './api/client';

function renderView(input: { client: SweeperClient; settings?: Settings | null }) {
  return render(<AnalyzerView client={input.client} settings={input.settings ?? ({ allowDeleteUnknown: false } as Settings)} onOpenLog={() => {}} />);
}

function rootNode() {
  return { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '0', itemCount: '0', risk: 'Safe' as const, description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [] as any[] };
}

async function runProgressCase() {
  const user = userEvent.setup();
  const longPath = 'C:\\Users\\Alice\\AppData\\Local\\Temp\\sweeper-sandbox-FwSDTi\\target\\d0000\\d0001\\d0002\\d0003\\very-long-unbroken-directory-name-without-spaces-for-overflow-testing';
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const client = {
    startAnalyzer: async (_root: any, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
      onProgress?.({ sessionId: 'a1', scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: 'C:\\' });
      onProgress?.({ sessionId: 'a1', scannedDirs: '42', scannedFiles: '7', scannedBytes: '4096', currentPath: longPath });
      await gate;
      return { sessionId: 'a1', rootPath: 'C:\\', totalBytes: '0', totalItems: '0', scannedDirs: '42', scannedFiles: '7', cancelled: false, warnings: [], root: rootNode() };
    },
  } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  const notice = await screen.findByRole('status');
  expect(notice).toHaveClass('analyzer-progress-notice');
  expect(notice.querySelector('.analyzer-progress-path')).toHaveAttribute('title', longPath);
  release();
  await waitFor(() => expect(screen.queryByText(/Analyzing…/)).not.toBeInTheDocument());
}

function setClipboard(input: { textRef: { value: string } }) {
  const original = navigator.clipboard;
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (text: any) => { input.textRef.value = String(text); } } });
  return () => Object.defineProperty(navigator, 'clipboard', { configurable: true, value: original });
}

async function runVhdxCase() {
  const user = userEvent.setup();
  const textRef = { value: '' };
  const restore = setClipboard({ textRef });
  const maliciousPath = "C:\\VMs\\evil'$(Start-Process calc).vhdx";
  const analyzerResult = { sessionId: 'an-vhdx', rootPath: 'C:\\', totalBytes: '10', totalItems: '1', scannedDirs: '1', scannedFiles: '1', cancelled: false, warnings: [], root: { ...rootNode(), totalBytes: '10', totalItems: '1', children: [{ id: 'v1', name: "evil'$(Start-Process calc).vhdx", path: maliciousPath, isDir: false, sizeBytes: '10', itemCount: '1', risk: 'Caution', description: 'vhdx', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: true, selectable: false, children: [] }] } } as StartAnalyzerResult;
  const client = { startAnalyzer: async () => analyzerResult, explainItem: async (itemId: any) => { let title = 'root'; if (String(itemId) === 'v1') title = "evil'$(Start-Process calc).vhdx"; return { itemId, title, description: 'desc', risk: 'Caution' as const }; } } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  const vhdxRow = (await screen.findAllByText(/\.vhdx$/i))[0].closest('[role="treeitem"]') as HTMLElement;
  await user.click(vhdxRow);
  const expectedCmd = "wsl --shutdown\nOptimize-VHD -Path 'C:\\VMs\\evil''$(Start-Process calc).vhdx' -Mode Full";
  expect(screen.getByText(/Optimize-VHD -Path/).closest('code')?.textContent).toBe(expectedCmd);
  await user.click(screen.getByRole('button', { name: 'Copy Command' }));
  await waitFor(() => expect(textRef.value).toBe(expectedCmd));
  restore();
}

async function runPresetCase() {
  const user = userEvent.setup();
  const analyzerResult = { sessionId: 'an-preset', rootPath: 'C:\\Users', totalBytes: '0', totalItems: '0', scannedDirs: '0', scannedFiles: '0', cancelled: false, warnings: [], root: { ...rootNode(), name: 'C:\\Users', path: 'C:\\Users' } } as StartAnalyzerResult;
  const startAnalyzer = vi.fn(async (path: any) => ({ ...analyzerResult, rootPath: path ?? analyzerResult.rootPath, root: { ...analyzerResult.root, name: path ?? analyzerResult.root.name, path: path ?? analyzerResult.root.path } }));
  const client = { startAnalyzer, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Use preset root path' }));
  const rootInput = screen.getByLabelText('Analyzer root path') as HTMLInputElement;
  expect(['C:\\', 'C:\\Users', 'C:\\Windows\\Temp']).toContain(rootInput.value);
  expect(rootInput.value.toLowerCase()).not.toMatch(/\\users\\[^\\]/);
}

async function runSmartQuoteCase() {
  const user = userEvent.setup();
  const maliciousPath = "C:\\VMs\\evil'\u2018\u2019\u201A\u201B$(calc).vhdx";
  const escapedPath = "C:\\VMs\\evil''\u2018\u2018\u2019\u2019\u201A\u201A\u201B\u201B$(calc).vhdx";
  const analyzerResult = { sessionId: 'an-vhdx-smart', rootPath: 'C:\\', totalBytes: '10', totalItems: '1', scannedDirs: '1', scannedFiles: '1', cancelled: false, warnings: [], root: { ...rootNode(), children: [{ id: 'v1', name: 'evil.vhdx', path: maliciousPath, isDir: false, sizeBytes: '10', itemCount: '1', risk: 'Caution', description: 'vhdx', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: true, selectable: false, children: [] }] } } as StartAnalyzerResult;
  const client = { startAnalyzer: async () => analyzerResult, explainItem: async (itemId: any) => ({ itemId, title: 'evil.vhdx', description: 'desc', risk: 'Caution' as const }) } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  const vhdxRow = (await screen.findAllByText(/evil\.vhdx/i))[0].closest('[role="treeitem"]') as HTMLElement;
  await user.click(vhdxRow);
  const expectedCmd = `wsl --shutdown\nOptimize-VHD -Path '${escapedPath}' -Mode Full`;
  await waitFor(() => expect(screen.getByText(/Optimize-VHD -Path/).closest('code')?.textContent).toBe(expectedCmd));
}

describe('AnalyzerView behavior', () => {
  it('renders analyzer long-path progress in bounded overflow-safe container (Bug 3a)', async () => { await runProgressCase(); });
  it('AnalyzerView escapes VHDX PowerShell path as single-quoted literal in render and copy', async () => { await runVhdxCase(); });
  it('AnalyzerView exposes Use Preset with a neutral root preset', async () => { await runPresetCase(); });
  it('AnalyzerView escapes ASCII and typographic smart single quotes in VHDX PowerShell command', async () => { await runSmartQuoteCase(); });
});