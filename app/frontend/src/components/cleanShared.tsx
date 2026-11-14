import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { RiskDto } from '../contract/bindings';
import { InfoIcon } from './Icons';

export type ExplainState = { itemId: string; title: string; description: string; risk: RiskDto } | null;
export type PreviewResult = {
  previewToken: string;
  breakdown: { targetId: string; targetName: string; selectedCount: string; selectedBytes: string }[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  issuedAtMs: number;
};

export const SESSION_TTL_MS = 15 * 60 * 1000;
export const PREVIEW_TTL_MS = 10 * 60 * 1000;
export const TYPED_CONFIRMATION_PHRASE = 'DELETE';

export function riskClass(risk: RiskDto): string {
  return `badge risk-${risk}`;
}

export function InfoTip({ label, children, align = 'left' }: { label: string; children: ReactNode; align?: 'left' | 'right' }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    if (!open) return;
    function onDocPointerDown(e: MouseEvent | PointerEvent) {
      const root = rootRef.current;
      if (!root) return;
      if (e.target instanceof Node && !root.contains(e.target)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        setOpen(false);
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
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => {
        if (rootRef.current?.contains(document.activeElement)) return;
        setOpen(false);
      }}
      onFocus={() => setOpen(true)}
      onBlur={(e) => {
        if (e.relatedTarget instanceof Node && rootRef.current?.contains(e.relatedTarget)) return;
        setOpen(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          setOpen(false);
        }
      }}
    >
      <button
        type="button"
        className="info-tip-btn"
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
      >
        <InfoIcon />
      </button>
      {open && (
        <span className="info-tip-popover" role="tooltip">
          {children}
        </span>
      )}
    </span>
  );
}

export function TtlIndicator({
  label,
  issuedAtMs,
  ttlMs,
  tooltip,
}: {
  label: string;
  issuedAtMs: number;
  ttlMs: number;
  tooltip?: string;
}) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, []);
  const remaining = Math.max(0, ttlMs - (Date.now() - issuedAtMs));
  const mins = Math.floor(remaining / 60000);
  const secs = Math.floor((remaining % 60000) / 1000);
  return (
    <span className="meta-chip num">
      <span>{label}: {mins}:{secs.toString().padStart(2, '0')}</span>
      {tooltip && (
        <InfoTip label={`${label} info`} align="right">
          <p>{tooltip}</p>
        </InfoTip>
      )}
    </span>
  );
}

export function ExplainDrawer({ explain, onClose }: { explain: ExplainState; onClose: () => void }) {
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (explain) closeBtnRef.current?.focus();
  }, [explain]);
  if (!explain) return null;
  return (
    <div className="drawer-backdrop" role="presentation" onClick={onClose}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="explain-title"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
        }}
      >
        <div className="row drawer-header">
          <h3 id="explain-title">Explain: {explain.title}</h3>
          <div className="spacer" />
          <button ref={closeBtnRef} className="btn" onClick={onClose} aria-label="Close explanation">Close</button>
        </div>
        <p className="drawer-risk"><span className={riskClass(explain.risk)}>Risk: {explain.risk}</span></p>
        <p className="drawer-description">{explain.description}</p>
        <div className="drawer-meta-card">
          <dl className="kv drawer-kv">
            <dt>Item ID</dt><dd><code>{explain.itemId}</code></dd>
          </dl>
        </div>
      </aside>
    </div>
  );
}
