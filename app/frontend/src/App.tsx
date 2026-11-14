import { useEffect, useLayoutEffect, useMemo, useState } from 'react';
import sweeperIcon from './assets/sweeper-icon.png';
import './styles/tokens.css';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { AnalyzerIcon, LogsIcon, MoonIcon, ScanIcon, SettingsIcon, ShieldIcon, SunIcon } from './components/Icons';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';

type View = 'scan' | 'analyzer' | 'settings' | 'logs';
type Theme = 'dark' | 'light';

function readStoredTheme(): Theme {
  if (typeof window === 'undefined') return 'dark';
  return window.localStorage.getItem('sweeper-theme') === 'light' ? 'light' : 'dark';
}

export function App() {
  const client = useMemo(() => createClient(), []);
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [elevationStatus, setElevationStatus] = useState<ElevationStatus>({ isElevated: false });
  const [elevationError, setElevationError] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(() => readStoredTheme());

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

  const elevationAria = elevationStatus.isElevated ? 'Administrator (Elevated)' : 'Standard (Non-Elevated)';
  const privilegeText = elevationStatus.isElevated ? 'Administrator' : 'Standard User';

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
              <button aria-current={view === 'scan' ? 'page' : undefined} onClick={() => setView('scan')}>
                <ScanIcon className="nav-icon" />
                <span>Scan &amp; Clean</span>
              </button>
              <button aria-current={view === 'analyzer' ? 'page' : undefined} onClick={() => setView('analyzer')}>
                <AnalyzerIcon className="nav-icon" />
                <span>Analyzer</span>
              </button>
            </div>
            <div className="nav-group">
              <div className="nav-group-label">System</div>
              <button aria-current={view === 'settings' ? 'page' : undefined} onClick={() => setView('settings')}>
                <SettingsIcon className="nav-icon" />
                <span>Settings</span>
              </button>
              <button aria-current={view === 'logs' ? 'page' : undefined} onClick={() => setView('logs')}>
                <LogsIcon className="nav-icon" />
                <span>Activity</span>
              </button>
            </div>
          </nav>
        </div>
        <div className="sidebar-footer sidebar-footer-compact">
          <div className="sidebar-user-row">
            <div
              className={`privilege-status ${elevationStatus.isElevated ? 'elevated' : ''}`}
              aria-label={elevationAria}
              title={elevationAria}
            >
              <ShieldIcon className="privilege-icon" />
              <span>{privilegeText}</span>
            </div>
            <div className="sidebar-theme-icons" role="group" aria-label="Theme">
              <button
                type="button"
                className={`icon-only-btn subtle theme-icon-btn ${theme === 'light' ? 'active' : ''}`}
                aria-label="Light"
                aria-pressed={theme === 'light'}
                title="Light"
                onClick={() => setTheme('light')}
              >
                <SunIcon />
              </button>
              <button
                type="button"
                className={`icon-only-btn subtle theme-icon-btn ${theme === 'dark' ? 'active' : ''}`}
                aria-label="Dark"
                aria-pressed={theme === 'dark'}
                title="Dark"
                onClick={() => setTheme('dark')}
              >
                <MoonIcon />
              </button>
            </div>
          </div>
        </div>
      </aside>
      <div className="app-main">
        <main className="main-content">
          {view === 'scan' && <ScanAndPreviewView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onOpenLog={openLog} />}
          {view === 'analyzer' && <AnalyzerView client={client} settings={settings} onOpenLog={openLog} onOpenSettings={() => setView('settings')} />}
          {view === 'settings' && <SettingsView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onSaved={setSettings} theme={theme} onThemeChange={setTheme} />}
          {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} />}
        </main>
      </div>
    </div>
  );
}
