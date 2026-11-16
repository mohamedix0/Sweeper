import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import './styles/tokens.css';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { AppSidebar, type View } from './components/AppSidebar';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView, type ScanAndPreviewViewHandle } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';

type Theme = 'dark' | 'light';
export type SettingsFocusTarget = 'allowDeleteUnknown' | 'enableTier3' | null;
export type SettingsDeepLink = { focus: SettingsFocusTarget; prefillCustomPath: string | null };

function readStoredTheme(): Theme {
  if (typeof window === 'undefined') return 'dark';
  try { return window.localStorage.getItem('sweeper-theme') === 'light' ? 'light' : 'dark'; } catch { return 'dark'; }
}

function usePersistentTheme() {
  const [theme, setTheme] = useState<Theme>(() => readStoredTheme());
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { window.localStorage.setItem('sweeper-theme', theme); } catch { /* ignore */ }
  }, [theme]);
  return { theme, setTheme };
}

export function App() {
  const client = useMemo(() => createClient(), []);
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [deepLink, setDeepLink] = useState<SettingsDeepLink>({ focus: null, prefillCustomPath: null });
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [elevation, setElevation] = useState<ElevationStatus>({ isElevated: false });
  const [elevationError, setElevationError] = useState<string | null>(null);
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const { theme, setTheme } = usePersistentTheme();
  const scanRef = useRef<ScanAndPreviewViewHandle | null>(null);

  useEffect(() => { let c = false; client.getSettings().then((s) => { if (!c) setSettings(s); }).catch(() => {}); return () => { c = true; }; }, [client]);
  useEffect(() => {
    let c = false;
    client.getElevationStatus().then((s) => { if (!c) { setElevation(s); setElevationError(null); } }).catch((e) => { if (!c) { setElevation({ isElevated: false }); setElevationError(formatContractError(e)); } });
    return () => { c = true; };
  }, [client]);

  const switchView = (next: View) => { if (next !== 'settings') setDeepLink({ focus: null, prefillCustomPath: null }); setView(next); };
  const openSettings = (focus: SettingsFocusTarget = null, prefillCustomPath: string | null = null) => { setDeepLink({ focus, prefillCustomPath }); setView('settings'); };
  const openLog = (logId: string) => { setFocusLogId(logId); setView('logs'); };
  const scanAgainFromLogs = () => { setView('scan'); window.setTimeout(() => { void scanRef.current?.runScan(); }, 0); };
  const handleSaved = (next: Settings) => { setSettings(next); setSettingsVersion((v) => v + 1); };
  const runAsAdministrator = async () => {
    setRelaunching(true); setRelaunchError(null);
    try { const res = await client.relaunchElevated(); if (!res.launched) setRelaunchError('Administrator launch was not started.'); }
    catch (e) { setRelaunchError(formatContractError(e)); } finally { setRelaunching(false); }
  };

  return (
    <div className="app-shell">
      <AppSidebar view={view} onSwitchView={switchView} elevationAria={elevation.isElevated ? 'Administrator (Elevated)' : 'Standard (Non-Elevated)'} isElevated={elevation.isElevated} relaunching={relaunching} onRunAsAdministrator={() => void runAsAdministrator()} />
      <div className="app-main">
        <main className="main-content">
          {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
          {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}
          <div className="view-panel" hidden={view !== 'scan'} aria-hidden={view !== 'scan'}>
            <ScanAndPreviewView ref={scanRef} client={client} settings={settings} settingsVersion={settingsVersion} elevationStatus={elevation} elevationError={null} onOpenLog={openLog} onOpenSettings={(focus, prefillCustomPath) => openSettings(focus ?? null, prefillCustomPath ?? null)} />
          </div>
          <div className="view-panel" hidden={view !== 'analyzer'} aria-hidden={view !== 'analyzer'}>
            <AnalyzerView client={client} settings={settings} onOpenLog={openLog} onOpenSettings={(focus, prefillCustomPath) => openSettings(focus ?? null, prefillCustomPath ?? null)} />
          </div>
          {view === 'settings' && <SettingsView client={client} settings={settings} elevationStatus={elevation} elevationError={null} onSaved={handleSaved} theme={theme} onThemeChange={setTheme} focusTarget={deepLink.focus} prefillCustomPath={deepLink.prefillCustomPath} onFocusConsumed={() => setDeepLink({ focus: null, prefillCustomPath: null })} />}
          {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} onScanAgain={scanAgainFromLogs} />}
        </main>
      </div>
    </div>
  );
}
