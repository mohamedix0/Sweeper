import { useEffect, useMemo, useState } from 'react';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';

type View = 'scan' | 'analyzer' | 'settings' | 'logs';

export function App() {
  const client = useMemo(() => createClient(), []);
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [elevationStatus, setElevationStatus] = useState<ElevationStatus>({ isElevated: false });
  const [elevationError, setElevationError] = useState<string | null>(null);

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

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">Sweeper</div>
        <nav className="nav" aria-label="Primary">
          <button aria-current={view === 'scan' ? 'page' : undefined} onClick={() => setView('scan')}>Scan &amp; Preview</button>
          <button aria-current={view === 'analyzer' ? 'page' : undefined} onClick={() => setView('analyzer')}>Analyzer</button>
          <button aria-current={view === 'settings' ? 'page' : undefined} onClick={() => setView('settings')}>Settings</button>
          <button aria-current={view === 'logs' ? 'page' : undefined} onClick={() => setView('logs')}>Logs</button>
        </nav>
      </header>
      <main className="main">
        {view === 'scan' && <ScanAndPreviewView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onOpenLog={openLog} />}
        {view === 'analyzer' && <AnalyzerView client={client} settings={settings} onOpenLog={openLog} />}
        {view === 'settings' && <SettingsView client={client} settings={settings} elevationStatus={elevationStatus} elevationError={elevationError} onSaved={setSettings} />}
        {view === 'logs' && <LogsView client={client} focusLogId={focusLogId} onFocusConsumed={() => setFocusLogId(null)} />}
      </main>
    </div>
  );
}
