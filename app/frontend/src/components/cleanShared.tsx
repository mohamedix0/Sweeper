import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import type { ExecuteCleanResponse, RiskDto } from '../api/client';
import { formatBytes, normalizeDisplayPath, pluralize, relativeAge } from '../utils/format';
import { CopyIcon, FileIcon, FolderOpenIcon, FolderIcon, InfoIcon } from './Icons';

export type ExplainState = { itemId: string; title: string; description: string; risk: RiskDto } | null;
export type PreviewResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  issuedAtMs: number;
};

export const SESSION_TTL_MS = 15 * 60 * 1000;
export const TYPED_CONFIRMATION_PHRASE = 'DELETE';

export async function copyText(text: string) {
  try {
    await navigator.clipboard?.writeText(text);
  } catch {
    /* ignore */
  }
}

export function openContainingFolder(path: string) {
  const clean = normalizeDisplayPath(path).replace(/[\\/]+$/, '');
  const idx = clean.lastIndexOf('\\');
  const folder = idx > 2 ? clean.slice(0, idx) : clean || 'C:\\';
  try {
    window.open(`file:///${folder.replace(/\\/g, '/')}`, '_blank', 'noopener,noreferrer');
  } catch {
    /* ignore */
  }
}

export function formatModifiedAge(mtimeUnixSecs: string | null | undefined, ageEligible: boolean, minAgeHours?: number): string {
  const rel = relativeAge(mtimeUnixSecs ?? null);
  if (ageEligible) return rel === '—' ? 'Older than minimum age' : rel;
  if (minAgeHours == null) return rel === '—' ? 'Recent' : rel;
  return rel === '—' ? `Recent (< ${minAgeHours}h)` : `${rel} (< ${minAgeHours}h)`;
}

export function PathWell({ path }: { path: string }) {
  const fullPath = normalizeDisplayPath(path) || 'C:\\';
  const lastSep = Math.max(fullPath.lastIndexOf('\\'), fullPath.lastIndexOf('/'));
  const dirPart = lastSep > 0 ? `${fullPath.slice(0, lastSep + 1)}` : '';
  const leafPart = lastSep > 0 ? fullPath.slice(lastSep + 1) : fullPath;
  return (
    <div className="path-well path-well-compact" aria-label="Full path" title={fullPath}>
      <div className="path-well-lines mono">
        {dirPart ? <div className="path-well-dir path-well-line">{dirPart}</div> : null}
        <div className="path-well-file path-well-line">{leafPart || fullPath}</div>
      </div>
    </div>
  );
}

export function PathActions({ path }: { path: string }) {
  const fullPath = normalizeDisplayPath(path) || 'C:\\';
  return (
    <div className="inspector-path-actions" aria-label="Path actions">
      <button type="button" className="btn small subtle-btn" onClick={() => void copyText(fullPath)} aria-label="Copy path" title="Copy path">
        <CopyIcon />
        <span>Copy Path</span>
      </button>
      <button type="button" className="btn small subtle-btn" onClick={() => openContainingFolder(fullPath)} aria-label="Reveal in Explorer" title="Reveal in Explorer">
        <FolderOpenIcon />
        <span>Reveal in Explorer</span>
      </button>
    </div>
  );
}

export function InspectorMetaGrid({
  size,
  modifiedAge,
  eligibility,
  extraLabel,
  extraValue,
}: {
  size: string;
  modifiedAge?: string | null;
  safety?: string;
  eligibility: string;
  extraLabel?: string;
  extraValue?: ReactNode;
}) {
  const secondLabel = modifiedAge != null ? 'Modified' : (extraLabel ?? 'Details');
  const secondValue = modifiedAge != null ? modifiedAge : (extraValue ?? '—');
  return (
    <dl className="inspector-meta-grid inspector-meta-grid-v19" aria-label="Item metadata">
      <div><dt>Size</dt><dd className="num">{formatBytes(size)}</dd></div>
      <div><dt>{secondLabel}</dt><dd>{secondValue}</dd></div>
      <div className="inspector-meta-eligibility inspector-meta-eligibility-full"><dt>Eligibility</dt><dd>{eligibility}</dd></div>
    </dl>
  );
}

export function InspectorTitleRow({
  icon,
  title,
  subtitle,
  badge,
}: {
  icon: ReactNode;
  title: string;
  subtitle?: string | null;
  badge: ReactNode;
}) {
  return (
    <div className="inspector-header-block">
      <div className="inspector-title-row">
        <span className="inspector-leading-icon-wrap" aria-hidden="true">{icon}</span>
        <div className="inspector-title-copy">
          <span className="inspector-headline-name" title={title}>{title}</span>
          {subtitle ? <span className="inspector-subtitle muted">{subtitle}</span> : null}
        </div>
        {badge}
      </div>
    </div>
  );
}

export function PreviewBreakdownBlock({
  preview,
  cleaning,
  cleanProgressLabel,
}: {
  preview: PreviewResult;
  cleaning: boolean;
  cleanProgressLabel: string | null;
}) {
  return (
    <div aria-live="polite" className="preview-breakdown">
      <h4 className="preview-breakdown-title">Preview Breakdown</h4>
      <p className="preview-total"><strong>Total selected:</strong> {pluralize(Number(preview.totalSelectedCount || '0'), 'item', 'items')}, <span className="num">{formatBytes(preview.totalSelectedBytes)}</span></p>
      <table className="table" aria-label="Preview breakdown by Target">
        <thead><tr><th>Target</th><th>Count</th><th>Size</th></tr></thead>
        <tbody>
          {preview.breakdown.map((b) => (
            <tr key={b.targetId}><td>{b.targetName}</td><td>{b.selectedCount}</td><td>{formatBytes(b.selectedBytes)}</td></tr>
          ))}
        </tbody>
      </table>
      <div className="clean-footer" role="alert" aria-live="polite">
        <div className="clean-footer-warning"><strong>Permanent deletion (bypasses Recycle Bin):</strong> Cleaning removes the selected eligible items permanently. This cannot be undone.</div>
      </div>
      {cleaning && (
        <div className="notice info" role="status" aria-live="polite">
          <div>Cleaning in progress…</div>
          {cleanProgressLabel && <div>{cleanProgressLabel}</div>}
          <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
        </div>
      )}
    </div>
  );
}

export function CleanSummaryBlock({ summary, onOpenLog }: { summary: ExecuteCleanResponse; onOpenLog: (logId: string) => void }) {
  return (
    <div className="clean-summary-section" aria-labelledby="clean-summary-heading" role="status">
      <h4 id="clean-summary-heading" className="clean-summary-title">Clean Summary</h4>
      {summary.aborted && <div className="notice error" role="alert"><strong>Clean aborted.</strong> {summary.abortReason || 'The operation was aborted.'}</div>}
      <dl className="kv">
        <dt>Bytes freed</dt><dd className="num">{formatBytes(summary.bytesFreed)}</dd>
        <dt>Deleted count</dt><dd className="num">{summary.deletedCount}</dd>
        <dt>Skipped count</dt><dd className="num">{summary.skippedCount}</dd>
      </dl>
      <div>
        <strong className="subheading-xs">Skipped reasons</strong>
        {summary.skippedReasons.length === 0 ? <p className="muted small-note" aria-label="Skipped reasons">None.</p> : (
          <ul aria-label="Skipped reasons" className="skipped-reasons">{summary.skippedReasons.map((r, idx) => <li key={`${r.reason}-${idx}`}>{r.reason} — {r.count}</li>)}</ul>
        )}
      </div>
      {summary.logId && <div className="row row-top"><button className="btn" onClick={() => onOpenLog(summary.logId!)}>View Operation Log</button></div>}
    </div>
  );
}

export function InfoTip({ label, children, align = 'left' }: { label: string; children: ReactNode; align?: 'left' | 'right' }) {
  const [pinned, setPinned] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const open = pinned || hovered || focused;
  const tooltipId = useId();
  const rootRef = useRef<HTMLSpanElement | null>(null);

  const close = () => {
    setPinned(false);
    setHovered(false);
    setFocused(false);
  };

  useEffect(() => {
    if (!open) return;
    function onDocPointerDown(e: MouseEvent | PointerEvent) {
      const root = rootRef.current;
      if (!root) return;
      if (e.target instanceof Node && !root.contains(e.target)) close();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        close();
      }
    }
    document.addEventListener('pointerdown', onDocPointerDown);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDocPointerDown);
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <span
      ref={rootRef}
      className={`info-tip ${align === 'right' ? 'right' : ''}`}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => {
        setHovered(false);
      }}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (e.relatedTarget instanceof Node && rootRef.current?.contains(e.relatedTarget)) return;
        setFocused(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          close();
        }
      }}
    >
      <button
        type="button"
        className="info-tip-btn"
        aria-label={label}
        aria-expanded={open}
        aria-describedby={open ? tooltipId : undefined}
        onClick={() => {
          if (pinned) {
            close();
          } else {
            setPinned(true);
          }
        }}
      >
        <InfoIcon />
      </button>
      {open && <span id={tooltipId} className="info-tip-popover" role="tooltip">{children}</span>}
    </span>
  );
}
