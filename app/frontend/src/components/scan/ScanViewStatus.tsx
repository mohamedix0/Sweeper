import { formatBytes } from '../../utils/format';

type Progress = { completedTargets: number; totalTargets: number; currentTargetName: string | null; scannedItems: string; scannedBytes: string } | null;

function scannedItemsLabel(progress: Progress): string {
  if (!progress) return '0';
  try { return BigInt(progress.scannedItems || '0').toLocaleString(); } catch { return progress.scannedItems || '0'; }
}

function noticeMessage(text: string, prefix?: string): string {
  if (prefix) return `${prefix}${text}`;
  return text;
}

function Notice(props: { kind: 'error' | 'info'; role: 'alert' | 'status'; text: string | null; prefix?: string }) {
  if (!props.text) return null;
  const msg = noticeMessage(props.text, props.prefix);
  return <div className={`notice ${props.kind}`} role={props.role}>{msg}</div>;
}

export function ScanNotices(props: { elevationError: string | null; relaunchInfo: string | null; relaunchError: string | null; error: string | null; info: string | null }) {
  return (
    <>
      <Notice kind="error" role="alert" text={props.elevationError} prefix="Elevation status unavailable: " />
      <Notice kind="info" role="status" text={props.relaunchInfo} />
      <Notice kind="error" role="alert" text={props.relaunchError} />
      <Notice kind="error" role="alert" text={props.error} />
      <Notice kind="info" role="status" text={props.info} />
    </>
  );
}

function hasTotals(progress: Progress): boolean {
  if (!progress) return false;
  return progress.totalTargets > 0;
}

function ScanProgressTargets(props: { progress: Progress }) {
  if (!hasTotals(props.progress)) return null;
  if (!props.progress) return null;
  return <span className="scan-progress-targets num" aria-label="Scan target progress">{props.progress.completedTargets} / {props.progress.totalTargets} Targets</span>;
}

function CurrentTarget(props: { name: string | null }) {
  if (!props.name) return null;
  return <span className="scan-progress-current">Current: <span className="scan-progress-current-name">{props.name}</span></span>;
}

function ScanProgressMeta(props: { progress: Progress }) {
  if (!props.progress) return null;
  const bytes = formatBytes({ bytes: props.progress.scannedBytes || '0' });
  return (
    <div className="scan-progress-meta">
      <CurrentTarget name={props.progress.currentTargetName} />
      <span className="scan-progress-counts num">{scannedItemsLabel(props.progress)} items · {bytes}</span>
    </div>
  );
}

function ScanProgressMeter(props: { progress: Progress }) {
  if (!hasTotals(props.progress) || !props.progress) return <div className="progress-bar progress-indeterminate" aria-hidden="true" />;
  return <progress className="scan-progress-meter" value={props.progress.completedTargets} max={props.progress.totalTargets} aria-label="Scan progress" />;
}

export function ScanProgressNotice(props: { scanStatus: string; scanProgress: Progress }) {
  return (
    <div role="status" aria-live="polite" className="notice info scanning-pulse scan-progress-notice">
      <div className="scan-progress-header">
        <span className="scan-progress-status">{props.scanStatus || 'Scanning…'}</span>
        <ScanProgressTargets progress={props.scanProgress} />
      </div>
      <ScanProgressMeta progress={props.scanProgress} />
      <div className="progress"><ScanProgressMeter progress={props.scanProgress} /></div>
    </div>
  );
}

function isFailureStatus(status: string): boolean {
  const lower = status.toLowerCase();
  if (lower.includes('failed')) return true;
  return lower.includes('cancelled');
}

export function ScanTerminalStatus(props: { scanStatus: string }) {
  if (isFailureStatus(props.scanStatus)) return <div className="notice error" role="alert">{props.scanStatus}</div>;
  return <div className="notice info" role="status">{props.scanStatus}</div>;
}

export function ScanOnboarding() {
  return (
    <section className="empty-state" aria-label="Scan onboarding">
      <h3>Ready to scan</h3>
      <p>Choose scope above, then run <strong>Start Scan</strong>. Nothing is removed until you Preview and confirm <strong>Clean Permanently</strong>.</p>
    </section>
  );
}
