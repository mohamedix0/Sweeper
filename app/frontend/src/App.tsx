import { useEffect, useMemo, useState } from 'react';
import sweeperIcon from './assets/sweeper-icon.png';
import './styles/tokens.css';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { LogsIcon, ScanIcon, AnalyzerIcon, SettingsIcon } from './components/Icons';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';

type View = 'scan' | 'analyzer' | 'settings' | 'logs';
type Theme = 'dark' | 'light';

const VIEW_LABELS: Record<View, string> = {
  scan: 'Scan & Preview',
  analyzer: 'Analyzer',
  settings: 'Settings',
  logs: 'Logs',
};

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

  useEffect(() => {
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

  const elevationLabel = elevationStatus.isElevated ? 'Elevated (Administrator)' : 'Standard (Non-Elevated)';
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
              <button aria-current={view === 'scan' ? 'page' : undefined} onClick={() => setView('scan')}>
                <ScanIcon className="nav-icon" />
                <span>Scan &amp; Preview</span>
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
                <span>Logs</span>
              </button>
            </div>
          </nav>
        </div>
        <div className="sidebar-footer">
          <div className="theme-toggle" role="group" aria-label="Theme">
            <button
              type="button"
              className={theme === 'dark' ? 'theme-segment active' : 'theme-segment'}
              aria-pressed={theme === 'dark'}
              onClick={() => setTheme('dark')}
            >
              Dark
            </button>
            <button
              type="button"
              className={theme === 'light' ? 'theme-segment active' : 'theme-segment'}
              aria-pressed={theme === 'light'}
              onClick={() => setTheme('light')}
            >
              Light
            </button>
          </div>
          <div className="elevation-badge-wrap">
            <span className={`badge ${elevationStatus.isElevated ? 'risk-Safe' : 'risk-Caution'}`} aria-label={elevationAria}>
              {elevationLabel}
            </span>
          </div>
        </div>
      </aside>
      <div className="app-main">
        <header className="main-topbar" aria-label="Context">
          <div className="breadcrumb">
            <span className="crumb-root">Sweeper</span>
            <span className="crumb-sep" aria-hidden="true">/</span>
            <span className="crumb-current">{VIEW_LABELS[view]}</span>
          </div>
          <div id="main-topbar-actions" className="topbar-actions" />
        </header>
        <main className="main-content">
          {view === 'scan' && <ScanAndPreviewView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onOpenLog={openLog} />}
          {view === 'analyzer' && <AnalyzerView client={client} settings={settings} onOpenLog={openLog} />}
          {view === 'settings' && <SettingsView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onSaved={setSettings} />}
          {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} />}
        </main>
      </div>
    </div>
  );
}
