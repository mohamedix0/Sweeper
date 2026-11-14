import { useEffect, useState } from 'react';
import type { RiskDto } from '../contract/bindings';

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

export function TtlIndicator({ label, issuedAtMs, ttlMs }: { label: string; issuedAtMs: number; ttlMs: number }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((x) => x + 1), 1000);
    return () => clearInterval(id);
  }, []);
  const remaining = Math.max(0, ttlMs - (Date.now() - issuedAtMs));
  const mins = Math.floor(remaining / 60000);
  const secs = Math.floor((remaining % 60000) / 1000);
  return (
    <span className="muted num">
      {label}: {mins}:{secs.toString().padStart(2, '0')} remaining
    </span>
  );
}

export function ExplainDrawer({ explain, onClose }: { explain: ExplainState; onClose: () => void }) {
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
        <div className="row">
          <h3 id="explain-title">Explain: {explain.title}</h3>
          <div className="spacer" />
          <button className="btn" onClick={onClose} aria-label="Close explanation">Close</button>
        </div>
        <p><span className={riskClass(explain.risk)}>Risk: {explain.risk}</span></p>
        <p>{explain.description}</p>
        <dl className="kv">
          <dt>Item ID</dt><dd><code>{explain.itemId}</code></dd>
        </dl>
      </aside>
    </div>
  );
}
