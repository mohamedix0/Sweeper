import type { Settings } from '../../api/client';
import { CheckIcon, LockIcon } from '../Icons';

export function SettingsSystemCachesCard(props: {
  draft: Settings;
  setDraft: (s: Settings) => void;
  deepDisabled: boolean;
  elevationStatus: { isElevated: boolean };
  relaunching: boolean;
  onRunAsAdmin: () => void;
}) {
  const { draft, setDraft, deepDisabled, elevationStatus, relaunching, onRunAsAdmin } = props;
  return (
    <section className="settings-group settings-group-linked" aria-labelledby="system-wide-caches-heading">
      <h2 id="system-wide-caches-heading" className="settings-group-title">System-Wide Windows Caches</h2>
      <div className="settings-group-card">
        <div className="settings-list-row settings-list-row-admin-cache settings-list-row-stack">
          <div className="settings-row-main settings-row-main-admin-cache">
            <div className="settings-row-copy">
              <h3>Allow system-wide Windows caches</h3>
              <p>Clean OS-wide shared caches such as <code>C:\Windows\Temp</code>.</p>
              {(!elevationStatus.isElevated || deepDisabled) && <p className="settings-helper-text">Deep Cleanup must be enabled first</p>}
            </div>
            <div className="settings-row-control">
              <label className="checkbox tier-toggle" title={!elevationStatus.isElevated || deepDisabled ? 'Enable Deep Cleanup and Administrator privileges first' : undefined}>
                <input
                  type="checkbox"
                  aria-label="Allow system-wide Windows caches (Admin)"
                  checked={draft.enableElevationTargets}
                  onChange={(e) => setDraft({ ...draft, enableElevationTargets: e.target.checked })}
                  disabled={!elevationStatus.isElevated || deepDisabled || relaunching}
                />
                <span className="tier-switch" aria-hidden="true" />
              </label>
            </div>
          </div>
          <div className="settings-prereq-banner settings-prereq-strip" aria-label="System-wide cache prerequisites">
            <span className={`settings-prereq-chip ${draft.enableTier3 ? 'ok' : 'missing'}`}>
              {draft.enableTier3 ? <CheckIcon aria-hidden="true" /> : <LockIcon aria-hidden="true" />}
              <span>{draft.enableTier3 ? 'Deep Cleanup enabled' : 'Deep Cleanup disabled'}</span>
              {!draft.enableTier3 && (
                <button type="button" className="btn small subtle-btn settings-prereq-action" onClick={() => setDraft({ ...draft, enableTier3: true })} disabled={relaunching}>Enable Deep Cleanup</button>
              )}
            </span>
            <span className={`settings-prereq-chip ${elevationStatus.isElevated ? 'ok' : 'missing'}`}>
              {elevationStatus.isElevated ? <CheckIcon aria-hidden="true" /> : <LockIcon aria-hidden="true" />}
              <span>{elevationStatus.isElevated ? 'Administrator' : 'Administrator required'}</span>
              {!elevationStatus.isElevated && (
                <button className="btn small subtle-btn settings-prereq-action" onClick={onRunAsAdmin} disabled={relaunching} aria-label="Run as administrator">Elevate</button>
              )}
            </span>
          </div>
        </div>
      </div>
    </section>
  );
}
