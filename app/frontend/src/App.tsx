import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import sweeperIcon from './assets/sweeper-icon.png';
import './styles/tokens.css';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { AnalyzerIcon, LogsIcon, ScanIcon, SettingsIcon, ShieldIcon } from './components/Icons';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView, type ScanAndPreviewViewHandle } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';

type View = 'scan' | 'analyzer' | 'settings' | 'logs';
type Theme = 'dark' | 'light';
export type SettingsFocusTarget = 'allowDeleteUnknown' | 'enableTier3' | null;
export type SettingsDeepLink = { focus: SettingsFocusTarget; prefillCustomPath: string | null };

function readStoredTheme(): Theme {
  if (typeof window === 'undefined') return 'dark';
  return window.localStorage.getItem('sweeper-theme') === 'light' ? 'light' : 'dark';
}

export function App() {
  const client = useMemo(() => createClient(), []);
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsDeepLink, setSettingsDeepLink] = useState<SettingsDeepLink>({ focus: null, prefillCustomPath: null });
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [elevationStatus, setElevationStatus] = useState<ElevationStatus>({ isElevated: false });
  const [elevationError, setElevationError] = useState<string | null>(null);
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(() => readStoredTheme());
  const scanRef = useRef<ScanAndPreviewViewHandle | null>(null);

  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
    window.localStorage.setItem('sweeper-theme', theme);
  }, [theme]);

  useEffect(() => {
    let cancelled = false;
    client.getSettings().then((s) => { if (!cancelled) setSettings(s); }).catch(() => { /* non-fatal */ });
    return () => { cancelled = true; };
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    client.getElevationStatus()
      .then((status) => {
        if (!cancelled) {
          setElevationStatus(status);
          setElevationError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setElevationStatus({ isElevated: false });
          setElevationError(formatContractError(e));
        }
      });
    return () => { cancelled = true; };
  }, [client]);

  function openLog(logId: string) {
    setFocusLogId(logId);
    setView('logs');
  }

  function scanAgainFromLogs() {
    setView('scan');
    window.setTimeout(() => { void scanRef.current?.runScan(); }, 0);
  }

  function openSettings(focus: SettingsFocusTarget = null, prefillCustomPath: string | null = null) {
    setSettingsDeepLink({ focus, prefillCustomPath });
    setView('settings');
  }

  function switchView(next: View) {
    if (next !== 'settings') setSettingsDeepLink({ focus: null, prefillCustomPath: null });
    setView(next);
  }

  function handleSettingsSaved(next: Settings) {
    setSettings(next);
    setSettingsVersion((v) => v + 1);
  }

  async function runAsAdministrator() {
    setRelaunching(true);
    setRelaunchError(null);
    try {
      const res = await client.relaunchElevated();
      if (!res.launched) setRelaunchError('Administrator launch was not started.');
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  }

  const elevationAria = elevationStatus.isElevated ? 'Administrator (Elevated)' : 'Standard (Non-Elevated)';

  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="Sidebar">
        <div className="sidebar-top">
          <div className="brand">
            <img className="brand-icon" src={sweeperIcon} alt="" width={24} height={24} />
            <span className="brand-text">Sweeper</span>
          </div>
          <nav className="nav" aria-label="Primary">
            <div className="nav-group">
              <div className="nav-group-label">Workspace</div>
              <button aria-current={view === 'scan' ? 'page' : undefined} onClick={(e) => { switchView('scan'); e.currentTarget.blur(); }}>
                <ScanIcon className="nav-icon" />
                <span>Scan &amp; Clean</span>
              </button>
              <button aria-current={view === 'analyzer' ? 'page' : undefined} onClick={(e) => { switchView('analyzer'); e.currentTarget.blur(); }}>
                <AnalyzerIcon className="nav-icon" />
                <span>Analyzer</span>
              </button>
            </div>
            <div className="nav-group">
              <div className="nav-group-label">System</div>
              <button aria-current={view === 'settings' ? 'page' : undefined} onClick={(e) => { switchView('settings'); e.currentTarget.blur(); }}>
                <SettingsIcon className="nav-icon" />
                <span>Settings</span>
              </button>
              <button aria-current={view === 'logs' ? 'page' : undefined} onClick={(e) => { switchView('logs'); e.currentTarget.blur(); }}>
                <LogsIcon className="nav-icon" />
                <span>Activity</span>
              </button>
            </div>
          </nav>
        </div>
        <div className="sidebar-footer">
          {!elevationStatus.isElevated ? (
            <div className="sidebar-priv-block sidebar-priv-compact">
              <span className="sr-only" aria-label={elevationAria}>{elevationAria}</span>
              <button
                type="button"
                className="sidebar-privilege-control"
                onClick={() => void runAsAdministrator()}
                disabled={relaunching}
                aria-label={relaunching ? 'Standard Mode · Requesting elevation' : 'Standard Mode · Elevate'} title="Restarts Sweeper as Administrator (preserves settings)"
              >
                <ShieldIcon className="privilege-icon" />
                <span>{relaunching ? 'Requesting…' : 'Standard Mode · Elevate'}</span>
              </button>
            </div>
          ) : (
            <div className="sidebar-priv-block sidebar-priv-compact">
              <div className="sidebar-privilege-control elevated" aria-label={elevationAria} title={`${elevationAria} · Running with administrator privileges`}>
                <ShieldIcon className="privilege-icon" />
                <span>Administrator</span>
              </div>
            </div>
          )}
        </div>
      </aside>
      <div className="app-main">
        <main className="main-content">
          {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
          {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}
          {view === 'scan' && <ScanAndPreviewView ref={scanRef} client={client} settings={settings} settingsVersion={settingsVersion} elevationStatus={elevationStatus} elevationError={null} onOpenLog={openLog} onOpenSettings={(focus, prefillCustomPath) => openSettings(focus ?? null, prefillCustomPath ?? null)} />}
          {view === 'analyzer' && <AnalyzerView client={client} settings={settings} onOpenLog={openLog} onOpenSettings={(focus, prefillCustomPath) => openSettings(focus ?? null, prefillCustomPath ?? null)} />}
          {view === 'settings' && <SettingsView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={null} onSaved={handleSettingsSaved} theme={theme} onThemeChange={setTheme} focusTarget={settingsDeepLink.focus} prefillCustomPath={settingsDeepLink.prefillCustomPath} onFocusConsumed={() => setSettingsDeepLink({ focus: null, prefillCustomPath: null })} />}
          {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} onScanAgain={scanAgainFromLogs} />}
        </main>
      </div>
    </div>
  );
}
