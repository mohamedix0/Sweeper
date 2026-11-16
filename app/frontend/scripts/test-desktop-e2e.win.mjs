import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

export function killProcessTree(pid) {
  if (pid) spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
}

export async function waitForExit(child, timeoutMs) {
  const done = child.exitCode !== null || child.signalCode !== null;
  if (done) return;
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), delay(timeoutMs)]);
}

function windowStateScript(pid) {
  return [
    '$ErrorActionPreference = "Stop";',
    'Add-Type @"',
    'using System; using System.Text; using System.Runtime.InteropServices;',
    'public static class Win {',
    '  public delegate bool EWP(IntPtr h, IntPtr p);',
    '  [DllImport("user32.dll")] public static extern bool EnumWindows(EWP cb, IntPtr p);',
    '  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);',
    '  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);',
    '  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder b, int m);',
    '}',
    '"@;',
    `$want=${pid}; $hit=$null;`,
    '$cb = [Win+EWP]{ param($h,$p) $proc=0; [Win]::GetWindowThreadProcessId($h,[ref]$proc)|Out-Null; if($proc -ne $want -or -not [Win]::IsWindowVisible($h)){return $true}; $sb=New-Object System.Text.StringBuilder 512; [Win]::GetWindowText($h,$sb,$sb.Capacity)|Out-Null; $t=$sb.ToString(); if($t -eq "Sweeper"){$script:hit=@($t,[int64]$h); return $false}; return $true };',
    '[Win]::EnumWindows($cb,[IntPtr]::Zero)|Out-Null;',
    'if($hit){ Write-Output ("{0}|{1}" -f $hit[0],$hit[1]) }',
  ].join('\n');
}

function spawnDetail(result) {
  const detail = String(result.stderr || '').trim();
  if (detail) return detail;
  return String(result.stdout || '').trim();
}

function readSweeperWindowState(pid) {
  const result = spawnSync('powershell', ['-NoProfile', '-Command', windowStateScript(pid)], { encoding: 'utf8' });
  if (result.status !== 0) return { ok: false, detail: spawnDetail(result) };
  const text = String(result.stdout || '').trim();
  if (!text) return { ok: false, title: '', handle: '0' };
  const [title = '', handle = '0'] = text.split('|');
  return { ok: title === 'Sweeper' && handle !== '0', title, handle };
}

function windowDetail(state) {
  if (state.detail) return state.detail;
  const title = state.title ?? '';
  const handle = state.handle ?? '0';
  return `title="${title}", handle=${handle}`;
}

export async function assertSweeperWindowVisible(pid, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let last = readSweeperWindowState(pid);
  while (Date.now() < deadline && !last.ok) {
    await delay(250);
    last = readSweeperWindowState(pid);
  }
  if (last.ok) return;
  throw new Error(`Sweeper window visibility check failed: ${windowDetail(last)}`);
}

export function resolveDesktopExePath() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const envPath = process.env.SWEEPER_DESKTOP_EXE;
  let exePath = path.resolve(scriptDir, '../../../target/debug/sweeper-desktop.exe');
  if (envPath) exePath = path.resolve(envPath);
  if (fs.existsSync(exePath)) return exePath;
  const repoRoot = path.resolve(scriptDir, '../../..');
  const targetDir = path.join(repoRoot, 'target');
  throw new Error(`sweeper-desktop executable not found at ${exePath}. Build it first with CARGO_TARGET_DIR set to the repository-root target directory so Cargo writes to <repo>/target/debug/ instead of app/target/debug/ (PowerShell: $env:CARGO_TARGET_DIR='${targetDir}'; cargo build --manifest-path app/Cargo.toml --package sweeper-desktop --locked).`);
}

function desktopEnv(port, tempRoot) {
  const webViewUserData = path.join(tempRoot, 'WebView2');
  const appData = path.join(tempRoot, 'AppData', 'Roaming');
  const localAppData = path.join(tempRoot, 'AppData', 'Local');
  fs.mkdirSync(appData, { recursive: true });
  fs.mkdirSync(localAppData, { recursive: true });
  fs.mkdirSync(webViewUserData, { recursive: true });
  return { ...process.env, SWEEPER_E2E_READ_ONLY: '1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: webViewUserData, APPDATA: appData, LOCALAPPDATA: localAppData };
}

export function spawnDesktop(exePath, port) {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'sweeper-desktop-e2e-'));
  const env = desktopEnv(port, tempRoot);
  const child = spawn(exePath, [], { cwd: path.dirname(exePath), env, stdio: 'ignore', windowsHide: false });
  return { child, tempRoot };
}

export function cleanupTemp(tempRoot) {
  try { fs.rmSync(tempRoot, { recursive: true, force: true }); } catch { /* best effort */ }
}