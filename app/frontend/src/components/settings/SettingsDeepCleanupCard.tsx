import type { Settings } from '../../api/client';
import type { Ref } from 'react';
import { LockIcon } from '../Icons';

export function SettingsDeepCleanupCard(props: {
  draft: Settings;
  setDraft: (s: Settings) => void;
  deepDisabled: boolean;
  highlightAllowUnknown: boolean;
  allowUnknownRowRef: Ref<HTMLDivElement>;
}) {
  const { draft, setDraft, deepDisabled, highlightAllowUnknown, allowUnknownRowRef } = props;
  return (
    <section className="settings-group" aria-labelledby="tier3-settings-heading">
      <h2 id="tier3-settings-heading" className="settings-group-title">Deep Cleanup &amp; Inactive Projects</h2>
      <div className="settings-group-card">
        <div className="settings-list-row">
          <div className="settings-row-copy">
            <h3>Enable Deep Cleanup Categories</h3>
            <p>Opt in to higher-risk cleanup categories. Protected Paths are never removable.</p>
            {deepDisabled && <span className="settings-lock-indicator"><LockIcon /> Enable Deep Cleanup above to configure inactive project rules</span>}
          </div>
          <div className="settings-row-control">
            <label className="checkbox tier-toggle">
              <input type="checkbox" aria-label="Enable deep cleanup categories" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} />
              <span className="tier-switch" aria-hidden="true" />
            </label>
          </div>
        </div>
        <div className={`settings-dependent-group settings-sub-rules ${deepDisabled ? 'is-disabled-group locked' : ''}`}>
          <div className={`settings-list-row settings-dependent-row settings-row--dependent settings-sub-rule ${deepDisabled ? 'is-disabled-row locked' : ''}`}>
            <div className="settings-row-copy">
              <h3>Inactive Node.js dependencies (node_modules)</h3>
              <p>Highlight inactive project caches when Deep Cleanup is enabled.</p>
            </div>
            <div className="settings-row-control settings-row-control-cluster">
              <label className="inline-threshold" htmlFor="stale-project-days-node">
                <input id="stale-project-days-node" className="compact-number" type="number" min={1} value={draft.staleProjectDays} onChange={(e) => setDraft({ ...draft, staleProjectDays: Number(e.target.value || 30) })} disabled={deepDisabled} aria-label="Inactive Node.js dependencies age" />
                <span className="unit-suffix">days</span>
              </label>
            </div>
          </div>
          <div className={`settings-list-row settings-dependent-row settings-row--dependent settings-sub-rule ${deepDisabled ? 'is-disabled-row locked' : ''}`}>
            <div className="settings-row-copy">
              <h3>Inactive Rust build artifacts (target)</h3>
              <p>Highlight inactive Rust build caches older than the threshold.</p>
            </div>
            <div className="settings-row-control settings-row-control-cluster">
              <label className="inline-threshold" htmlFor="stale-project-days">
                <input id="stale-project-days" className="compact-number" type="number" min={1} value={draft.staleProjectDays} onChange={(e) => setDraft({ ...draft, staleProjectDays: Number(e.target.value || 30) })} disabled={deepDisabled} aria-label="Inactive project cache age" />
                <span className="unit-suffix">days</span>
              </label>
            </div>
          </div>
          <div className={`settings-list-row settings-dependent-row settings-row--dependent settings-sub-rule ${deepDisabled ? 'is-disabled-row locked' : ''}`}>
            <div className="settings-row-copy">
              <h3>Empty Windows Recycle Bin</h3>
              <p>Included with Deep Cleanup (shared <code>enableTier3</code> setting). When Deep Cleanup is enabled, cleaning can permanently empty the Recycle Bin.</p>
            </div>
            <div className="settings-row-control settings-row-control-cluster">
              <span className={`badge risk-Caution${deepDisabled ? ' badge-muted-locked' : ''}`}>Caution · Permanent</span>
              <span className="muted small-note" aria-label="Recycle Bin included with Deep Cleanup">
                {draft.enableTier3 ? 'Included with Deep Cleanup' : 'Controlled by Deep Cleanup'}
              </span>
            </div>
          </div>
        </div>
        <div ref={allowUnknownRowRef} className={`settings-list-row settings-focusable-row${highlightAllowUnknown ? ' settings-row-highlight' : ''}`}>
          <div className="settings-row-copy">
            <h3>Allow deleting unknown folders <span className="badge risk-Unknown settings-inline-badge">Requires DELETE confirm</span></h3>
            <p>Advanced. Unknown folders always require typed <code>DELETE</code> confirmation.</p>
          </div>
          <div className="settings-row-control">
            <label className="checkbox tier-toggle">
              <input type="checkbox" aria-label="Allow deleting unknown folders (advanced)" checked={draft.allowDeleteUnknown} onChange={(e) => setDraft({ ...draft, allowDeleteUnknown: e.target.checked })} />
              <span className="tier-switch" aria-hidden="true" />
            </label>
          </div>
        </div>
      </div>
    </section>
  );
}
