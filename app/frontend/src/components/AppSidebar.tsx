import sweeperIcon from '../assets/sweeper-icon.png';
import { AnalyzerIcon, LogsIcon, ScanIcon, SettingsIcon, ShieldIcon } from './Icons';

export type View = 'scan' | 'analyzer' | 'settings' | 'logs';

export function AppSidebar(props: {
  view: View;
  onSwitchView: (next: View) => void;
  elevationAria: string;
  isElevated: boolean;
  relaunching: boolean;
  onRunAsAdministrator: () => void;
}) {
  return (
    <aside className="sidebar" aria-label="Sidebar">
      <div className="sidebar-top">
        <div className="brand">
          <img className="brand-icon" src={sweeperIcon} alt="" width={24} height={24} />
          <span className="brand-text">Sweeper</span>
        </div>
        <nav className="nav" aria-label="Primary">
          <div className="nav-group">
            <div className="nav-group-label">Workspace</div>
            <button aria-current={props.view === 'scan' ? 'page' : undefined} onClick={(e) => { props.onSwitchView('scan'); e.currentTarget.blur(); }}>
              <ScanIcon className="nav-icon" /><span>Scan &amp; Clean</span>
            </button>
            <button aria-current={props.view === 'analyzer' ? 'page' : undefined} onClick={(e) => { props.onSwitchView('analyzer'); e.currentTarget.blur(); }}>
              <AnalyzerIcon className="nav-icon" /><span>Analyzer</span>
            </button>
          </div>
          <div className="nav-group">
            <div className="nav-group-label">System</div>
            <button aria-current={props.view === 'settings' ? 'page' : undefined} onClick={(e) => { props.onSwitchView('settings'); e.currentTarget.blur(); }}>
              <SettingsIcon className="nav-icon" /><span>Settings</span>
            </button>
            <button aria-current={props.view === 'logs' ? 'page' : undefined} onClick={(e) => { props.onSwitchView('logs'); e.currentTarget.blur(); }}>
              <LogsIcon className="nav-icon" /><span>Activity</span>
            </button>
          </div>
        </nav>
      </div>
      <div className="sidebar-footer">
        {!props.isElevated ? (
          <div className="sidebar-priv-block sidebar-priv-compact">
            <span className="sr-only" aria-label={props.elevationAria}>{props.elevationAria}</span>
            <button type="button" className="sidebar-privilege-control" onClick={props.onRunAsAdministrator} disabled={props.relaunching} aria-label={props.relaunching ? 'Standard Mode · Requesting elevation' : 'Standard Mode · Elevate'} title="Restarts Sweeper as Administrator (preserves settings)">
              <ShieldIcon className="privilege-icon" />
              <span>{props.relaunching ? 'Requesting…' : 'Standard Mode · Elevate'}</span>
            </button>
          </div>
        ) : (
          <div className="sidebar-priv-block sidebar-priv-compact">
            <div className="sidebar-privilege-control elevated" aria-label={props.elevationAria} title={`${props.elevationAria} · Running with administrator privileges`}>
              <ShieldIcon className="privilege-icon" /><span>Administrator</span>
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
