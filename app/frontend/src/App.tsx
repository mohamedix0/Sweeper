import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import './styles/tokens.css';
import './App.css';
import { createClient, formatContractError, type ElevationStatus, type Settings } from './api/client';
import { AnalyzerView } from './components/AnalyzerView';
import { AppSidebar, type View } from './components/AppSidebar';
import { LogsView } from './components/LogsView';
import { ScanAndPreviewView, type ScanAndPreviewViewHandle } from './components/ScanAndPreviewView';
import { SettingsView } from './components/SettingsView';
import { UnsavedSettingsDialog } from './components/UnsavedSettingsDialog';

type Theme = 'dark' | 'light';
export type SettingsFocusTarget = 'allowDeleteUnknown' | 'enableTier3' | null;
export type SettingsDeepLink = { focus: SettingsFocusTarget; prefillCustomPath: string | null };

function themeFromStorage(value: string | null): Theme {
  if (value === 'light') return 'light';
  return 'dark';
}

function readStoredTheme(): Theme {
  if (typeof window === 'undefined') return 'dark';
  try {
    return themeFromStorage(window.localStorage.getItem('sweeper-theme'));
  } catch {
    return 'dark';
  }
}

function usePersistentTheme() {
  const [theme, setTheme] = useState<Theme>(() => readStoredTheme());
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      window.localStorage.setItem('sweeper-theme', theme);
    } catch {
      /* ignore */
    }
  }, [theme]);
  return { theme, setTheme };
}

function ElevationStatusNotice(props: { message: string | null }) {
  if (!props.message) return null;
  return <div className="notice error" role="alert">Elevation status unavailable: {props.message}</div>;
}

function RelaunchErrorNotice(props: { message: string | null }) {
  if (!props.message) return null;
  return <div className="notice error" role="alert">{props.message}</div>;
}

function ElevationNotices(props: { elevationError: string | null; relaunchError: string | null }) {
  return (
    <>
      <ElevationStatusNotice message={props.elevationError} />
      <RelaunchErrorNotice message={props.relaunchError} />
    </>
  );
}

interface ViewPanelsProps {
  view: View;
  client: ReturnType<typeof createClient>;
  settings: Settings | null;
  settingsVersion: number;
  elevation: ElevationStatus;
  scanRef: React.RefObject<ScanAndPreviewViewHandle>;
  onOpenLog: (logId: string) => void;
  onOpenSettings: (focus: SettingsFocusTarget, prefillCustomPath: string | null) => void;
  focusLogId: string | null;
  onFocusConsumed: () => void;
  onScanAgain: () => void;
  theme: Theme;
  setTheme: (t: Theme) => void;
  deepLink: SettingsDeepLink;
  onDeepLinkConsumed: () => void;
  onSaved: (s: Settings) => void;
  onSettingsDirtyChange: (dirty: boolean) => void;
}

function isHidden(view: View, name: View): boolean {
  return view !== name;
}

function HiddenPanel(props: { name: View; view: View; children: ReactNode }) {
  const hidden = isHidden(props.view, props.name);
  return (
    <div className="view-panel" hidden={hidden} aria-hidden={hidden}>
      {props.children}
    </div>
  );
}

function SettingsPanel(props: { view: View; client: ReturnType<typeof createClient>; settings: Settings | null; elevation: ElevationStatus; theme: Theme; setTheme: (t: Theme) => void; deepLink: SettingsDeepLink; onDeepLinkConsumed: () => void; onSaved: (s: Settings) => void; onDirtyChange: (dirty: boolean) => void }) {
  if (props.view !== 'settings') return null;
  return (
    <SettingsView
      client={props.client}
      settings={props.settings}
      elevationStatus={props.elevation}
      elevationError={null}
      onSaved={props.onSaved}
      theme={props.theme}
      onThemeChange={props.setTheme}
      focusTarget={props.deepLink.focus}
      prefillCustomPath={props.deepLink.prefillCustomPath}
      onFocusConsumed={props.onDeepLinkConsumed}
      onDirtyChange={props.onDirtyChange}
    />
  );
}

function LogsPanel(props: { view: View; client: ReturnType<typeof createClient>; focusLogId: string | null; onFocusConsumed: () => void; onScanAgain: () => void }) {
  if (props.view !== 'logs') return null;
  return <LogsView client={props.client} focusLogId={props.focusLogId} onFocusConsumed={props.onFocusConsumed} onScanAgain={props.onScanAgain} />;
}

function ScanPanel(props: { client: ReturnType<typeof createClient>; settings: Settings | null; settingsVersion: number; elevation: ElevationStatus; scanRef: React.RefObject<ScanAndPreviewViewHandle>; onOpenLog: (logId: string) => void; onOpenSettings: (focus: SettingsFocusTarget, prefillCustomPath: string | null) => void }) {
  return (
    <ScanAndPreviewView
      ref={props.scanRef}
      client={props.client}
      settings={props.settings}
      settingsVersion={props.settingsVersion}
      elevationStatus={props.elevation}
      elevationError={null}
      onOpenLog={props.onOpenLog}
      onOpenSettings={(focus, prefillCustomPath) => props.onOpenSettings(focus ?? null, prefillCustomPath ?? null)}
    />
  );
}

function AnalyzerPanel(props: { client: ReturnType<typeof createClient>; settings: Settings | null; onOpenLog: (logId: string) => void; onOpenSettings: (focus: SettingsFocusTarget, prefillCustomPath: string | null) => void }) {
  return (
    <AnalyzerView
      client={props.client}
      settings={props.settings}
      onOpenLog={props.onOpenLog}
      onOpenSettings={(focus, prefillCustomPath) => props.onOpenSettings(focus ?? null, prefillCustomPath ?? null)}
    />
  );
}

function ViewPanels(props: ViewPanelsProps) {
  return (
    <>
      <HiddenPanel name="scan" view={props.view}>
        <ScanPanel client={props.client} settings={props.settings} settingsVersion={props.settingsVersion} elevation={props.elevation} scanRef={props.scanRef} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      </HiddenPanel>
      <HiddenPanel name="analyzer" view={props.view}>
        <AnalyzerPanel client={props.client} settings={props.settings} onOpenLog={props.onOpenLog} onOpenSettings={props.onOpenSettings} />
      </HiddenPanel>
      <SettingsPanel view={props.view} client={props.client} settings={props.settings} elevation={props.elevation} theme={props.theme} setTheme={props.setTheme} deepLink={props.deepLink} onDeepLinkConsumed={props.onDeepLinkConsumed} onSaved={props.onSaved} onDirtyChange={props.onSettingsDirtyChange} />
      <LogsPanel view={props.view} client={props.client} focusLogId={props.focusLogId} onFocusConsumed={props.onFocusConsumed} onScanAgain={props.onScanAgain} />
    </>
  );
}

function useElevationState(client: ReturnType<typeof createClient>) {
  const [elevation, setElevation] = useState<ElevationStatus>({ isElevated: false });
  const [elevationError, setElevationError] = useState<string | null>(null);
  useEffect(() => {
    let c = false;
    client.getElevationStatus().then((s) => {
      if (!c) {
        setElevation(s);
        setElevationError(null);
      }
    }).catch((e) => {
      if (!c) {
        setElevation({ isElevated: false });
        setElevationError(formatContractError(e));
      }
    });
    return () => { c = true; };
  }, [client]);
  return { elevation, elevationError };
}

function elevationAria(args: { elevated: boolean }): string {
  if (args.elevated) return 'Administrator (Elevated)';
  return 'Standard (Non-Elevated)';
}

function useSettingsState(client: ReturnType<typeof createClient>) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsVersion, setSettingsVersion] = useState(0);
  useEffect(() => {
    let c = false;
    client.getSettings().then((s) => { if (!c) setSettings(s); }).catch(() => {});
    return () => { c = true; };
  }, [client]);
  const handleSaved = (next: Settings) => {
    setSettings(next);
    setSettingsVersion((v) => v + 1);
  };
  return { settings, settingsVersion, handleSaved };
}

function useRelaunch(client: ReturnType<typeof createClient>) {
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const onRelaunchSuccess = (launched: boolean) => {
    if (launched) return;
    setRelaunchError('Administrator launch was not started.');
  };
  const runAsAdministrator = async () => {
    setRelaunching(true);
    setRelaunchError(null);
    try {
      const res = await client.relaunchElevated();
      onRelaunchSuccess(res.launched);
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  };
  return { relaunching, relaunchError, runAsAdministrator };
}

type PendingNavigation = {
  target: View;
  apply: () => void;
};

// Every view transition funnels through this hook. A single app-shell dirty
// flag (published by the mounted Settings view) is enough because leaving
// Settings unmounts its controller and with it the unsaved draft. Holding a
// transition until the user stays or explicitly discards now covers *every*
// leave-Settings path, not only the sidebar buttons.
export function useAppNavigation(scanRef: React.RefObject<ScanAndPreviewViewHandle>) {
  const [view, setView] = useState<View>('scan');
  const [focusLogId, setFocusLogId] = useState<string | null>(null);
  const [deepLink, setDeepLink] = useState<SettingsDeepLink>({ focus: null, prefillCustomPath: null });
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [pending, setPending] = useState<PendingNavigation | null>(null);

  const clearDeepLink = () => setDeepLink({ focus: null, prefillCustomPath: null });

  // The one choke point for all transitions. Only transitions that leave a
  // dirty Settings draft are held; the full side effect is captured so a
  // discard completes exactly the transition the user asked for. Transitions
  // into Settings (deep links) are never blocked or deferred.
  const transitionTo = (target: View, apply: () => void) => {
    if (view === 'settings' && settingsDirty && target !== 'settings') {
      setPending({ target, apply });
      return;
    }
    apply();
  };

  const switchView = (next: View) => {
    transitionTo(next, () => {
      if (next !== 'settings') clearDeepLink();
      setView(next);
    });
  };

  const openSettings = (focus: SettingsFocusTarget = null, prefillCustomPath: string | null = null) => {
    transitionTo('settings', () => {
      setDeepLink({ focus, prefillCustomPath });
      setView('settings');
    });
  };

  const openLog = (logId: string) => {
    transitionTo('logs', () => {
      setFocusLogId(logId);
      setView('logs');
    });
  };

  const scanAgainFromLogs = () => {
    transitionTo('scan', () => {
      setView('scan');
      window.setTimeout(() => { void scanRef.current?.runScan(); }, 0);
    });
  };

  const keepEditing = () => setPending(null);

  const discardAndLeave = () => {
    const next = pending;
    setPending(null);
    setSettingsDirty(false);
    if (next) next.apply();
  };

  return {
    view, focusLogId, deepLink,
    switchView, openSettings, openLog, scanAgainFromLogs,
    clearDeepLink, clearFocusLog: () => setFocusLogId(null),
    reportDirty: setSettingsDirty,
    pendingView: pending?.target ?? null,
    keepEditing, discardAndLeave,
  };
}

function AppMain(props: { client: ReturnType<typeof createClient>; settings: Settings | null; settingsVersion: number; elevation: ElevationStatus; elevationError: string | null; relaunchError: string | null; scanRef: React.RefObject<ScanAndPreviewViewHandle>; nav: ReturnType<typeof useAppNavigation>; theme: Theme; setTheme: (t: Theme) => void; onSaved: (s: Settings) => void; onSettingsDirtyChange: (dirty: boolean) => void }) {
  return (
    <div className="app-main">
      <main className="main-content">
        <ElevationNotices elevationError={props.elevationError} relaunchError={props.relaunchError} />
        <ViewPanels
          view={props.nav.view}
          client={props.client}
          settings={props.settings}
          settingsVersion={props.settingsVersion}
          elevation={props.elevation}
          scanRef={props.scanRef}
          onOpenLog={props.nav.openLog}
          onOpenSettings={props.nav.openSettings}
          focusLogId={props.nav.focusLogId}
          onFocusConsumed={props.nav.clearFocusLog}
          onScanAgain={props.nav.scanAgainFromLogs}
          theme={props.theme}
          setTheme={props.setTheme}
          deepLink={props.nav.deepLink}
          onDeepLinkConsumed={props.nav.clearDeepLink}
          onSaved={props.onSaved}
          onSettingsDirtyChange={props.onSettingsDirtyChange}
        />
      </main>
    </div>
  );
}

export function App() {
  const client = useMemo(() => createClient(), []);
  const scanRef = useRef<ScanAndPreviewViewHandle>(null);
  const nav = useAppNavigation(scanRef);
  const relaunch = useRelaunch(client);
  const { theme, setTheme } = usePersistentTheme();
  const { elevation, elevationError } = useElevationState(client);
  const { settings, settingsVersion, handleSaved } = useSettingsState(client);
  return (
    <div className="app-shell">
      <AppSidebar
        view={nav.view}
        onSwitchView={nav.switchView}
        elevationAria={elevationAria({ elevated: elevation.isElevated })}
        isElevated={elevation.isElevated}
        relaunching={relaunch.relaunching}
        onRunAsAdministrator={() => void relaunch.runAsAdministrator()}
      />
      <AppMain client={client} settings={settings} settingsVersion={settingsVersion} elevation={elevation} elevationError={elevationError} relaunchError={relaunch.relaunchError} scanRef={scanRef} nav={nav} theme={theme} setTheme={setTheme} onSaved={handleSaved} onSettingsDirtyChange={nav.reportDirty} />
      {nav.pendingView && (
        <UnsavedSettingsDialog onKeepEditing={nav.keepEditing} onDiscard={nav.discardAndLeave} />
      )}
    </div>
  );
}
