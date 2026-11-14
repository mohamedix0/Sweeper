import { useEffect, useState } from 'react';
import { formatContractError, type ElevationStatus, type Settings, type SweeperClient } from '../api/client';
import { TYPED_CONFIRMATION_PHRASE } from './cleanShared';

function ElevationBadge({ isElevated }: { isElevated: boolean }) {
  return (
    <span className={`badge ${isElevated ? 'risk-Safe' : 'risk-Caution'}`} aria-label={isElevated ? 'Administrator (Elevated)' : 'Standard (Non-Elevated)'}>
      {isElevated ? 'Administrator (Elevated)' : 'Standard (Non-Elevated)'}
    </span>
  );
}

export function SettingsView({
  client,
  settings,
  elevationStatus,
  elevationError,
  onSaved,
}: {
  client: SweeperClient;
  settings: Settings | null;
  elevationStatus: ElevationStatus;
  elevationError: string | null;
  onSaved: (s: Settings) => void;
}) {
  const [draft, setDraft] = useState<Settings | null>(settings);
  const [loading, setLoading] = useState(!settings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [customPathInput, setCustomPathInput] = useState('');
  const [validatingPath, setValidatingPath] = useState(false);
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchInfo, setRelaunchInfo] = useState<string | null>(null);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);

  useEffect(() => {
    if (settings) {
      setDraft(settings);
      setLoading(false);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const s = await client.getSettings();
        if (!cancelled) {
          setDraft(s);
          onSaved(s);
        }
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, settings, onSaved]);

  async function save(next?: Settings) {
    const toSave = next ?? draft;
    if (!toSave) return;
    setSaving(true);
    setError(null);
    setInfo(null);
    try {
      const saved = await client.updateSettings(toSave);
      setDraft(saved);
      onSaved(saved);
      setInfo('Settings saved. Active sessions may be invalidated when scan-affecting settings change.');
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setSaving(false);
    }
  }

  async function addCustomPath() {
    if (!draft) return;
    const raw = customPathInput.trim();
    if (!raw) return;
    setValidatingPath(true);
    setError(null);
    setInfo(null);
    try {
      const verdict = await client.validateCustomPath(raw);
      if (!verdict.allowed) {
        setError(`Custom Path rejected: ${verdict.reason}`);
        return;
      }
      const normalized = verdict.path || raw;
      if (draft.customPaths.some((p) => p.toLowerCase() === normalized.toLowerCase())) {
        setError('Custom Path already added.');
        return;
      }
      const next = { ...draft, customPaths: [...draft.customPaths, normalized] };
      setCustomPathInput('');
      await save(next);
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setValidatingPath(false);
    }
  }

  async function removeCustomPath(path: string) {
    if (!draft) return;
    const next = { ...draft, customPaths: draft.customPaths.filter((p) => p !== path) };
    await save(next);
  }

  async function runAsAdministrator() {
    setRelaunching(true);
    setRelaunchInfo(null);
    setRelaunchError(null);
    try {
      const res = await client.relaunchElevated();
      if (res.launched) {
        setRelaunchInfo('Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.');
      } else {
        setRelaunchError('Administrator launch was not started.');
      }
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  }

  if (loading || !draft) {
    return (
      <section className="panel" aria-labelledby="settings-heading">
        <h2 id="settings-heading">Settings</h2>
        {error ? <div className="notice error" role="alert">{error}</div> : <p>Loading settings…</p>}
      </section>
    );
  }

  return (
    <section className="panel" aria-labelledby="settings-heading">
      <div className="row">
        <h2 id="settings-heading">Settings</h2>
        <div className="spacer" />
        <ElevationBadge isElevated={elevationStatus.isElevated} />
      </div>
      <p className="muted">Advanced settings are validated by the backend on load/save. Invalid or tampered values fall back to safe defaults and Protected Paths are never configurable.</p>
      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}
      {relaunchInfo && <div className="notice info" role="status">{relaunchInfo}</div>}
      {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}

      <div className="settings-section" aria-labelledby="general-settings-heading">
        <h3 id="general-settings-heading" className="section-title">General</h3>
        <div className="row">
          <label className="checkbox"><input type="checkbox" checked={draft.enableTier1} onChange={(e) => setDraft({ ...draft, enableTier1: e.target.checked })} /> Enable Tier 1</label>
          <label className="checkbox"><input type="checkbox" checked={draft.enableTier2} onChange={(e) => setDraft({ ...draft, enableTier2: e.target.checked })} /> Enable Tier 2</label>
        </div>
        <div className="field">
          <label htmlFor="min-age-hours">Minimum Age (hours)</label>
          <input id="min-age-hours" type="number" min={0} value={draft.minAgeHours} onChange={(e) => setDraft({ ...draft, minAgeHours: Number(e.target.value || 0) })} />
          {draft.minAgeHours < 24 && (
            <div className="notice warn" role="note">Warning: Minimum Age below 24 hours may include very recent files. Use with caution.</div>
          )}
        </div>
      </div>

      <div className="settings-section" aria-labelledby="tier3-settings-heading">
        <h3 id="tier3-settings-heading" className="section-title">Tier 3 (Opt-in, disabled by default)</h3>
        <div className="notice warn" role="note">Tier 3 targets are higher risk and require explicit opt-in. No Target may override Protected Paths.</div>
        <label className="checkbox"><input type="checkbox" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} /> Enable Tier 3 targets</label>
        <div className="field">
          <label htmlFor="stale-project-days">Stale project threshold (days)</label>
          <input id="stale-project-days" type="number" min={1} value={draft.staleProjectDays} onChange={(e) => setDraft({ ...draft, staleProjectDays: Number(e.target.value || 30) })} disabled={!draft.enableTier3} />
        </div>
        <div className="settings-section" aria-labelledby="elevation-targets-heading">
          <h4 id="elevation-targets-heading" className="section-title">Administrator-only Tier 3 targets</h4>
          <p className="muted">Windows Temp, Windows Update cache, Prefetch, and Package Cache require administrator elevation and Tier 3 opt-in. Standard startup never triggers UAC.</p>
          <label className="checkbox">
            <input
              type="checkbox"
              aria-label="Allow Administrator-only Tier 3 targets (Windows Temp, Update cache, Prefetch, Package Cache)"
              checked={draft.enableElevationTargets}
              onChange={(e) => setDraft({ ...draft, enableElevationTargets: e.target.checked })}
              disabled={!elevationStatus.isElevated || !draft.enableTier3}
            />
            Allow Administrator-only Tier 3 targets (Windows Temp, Update cache, Prefetch, Package Cache)
          </label>
          {!elevationStatus.isElevated && (
            <div className="notice warn" role="note">
              <div className="row">
                <span className="spacer">Requires administrator. Enable Tier 3 and run as administrator to unlock these targets.</span>
                <button className="btn" onClick={runAsAdministrator} disabled={relaunching}>Run as administrator</button>
                {relaunching && <span className="muted" role="status">Requesting administrator launch…</span>}
              </div>
            </div>
          )}
          {elevationStatus.isElevated && !draft.enableTier3 && <div className="notice" role="note">Enable Tier 3 to use Administrator-only targets.</div>}
          {elevationStatus.isElevated && (
            <div className="row row-top">
              <button className="btn" onClick={runAsAdministrator} disabled={relaunching}>Run as administrator</button>
              {relaunching && <span className="muted" role="status">Requesting administrator launch…</span>}
            </div>
          )}
        </div>
        <ul className="tier-notes" aria-label="Tier 3 target notes">
          <li><strong>Recycle Bin (opt-in permanent empty):</strong> empties Recycle Bin contents when Tier 3 is enabled (Caution).</li>
          <li><strong>Stale Rust <code>target/</code>:</strong> only when sibling <code>Cargo.toml</code> is unmodified for at least {draft.staleProjectDays} days.</li>
          <li><strong>Stale <code>node_modules/</code>:</strong> only when a sibling lockfile is present and stale by the configured threshold.</li>
          <li><strong>WSL/Docker <code>.vhdx</code>:</strong> <em>display-only</em> in Analyzer; never deleted.</li>
        </ul>
      </div>

      <div className="settings-section" aria-labelledby="unknown-settings-heading">
        <h3 id="unknown-settings-heading" className="section-title">Unknown Deletions</h3>
        <div className="notice warn" role="note">Allowing deletion of unknown folders is advanced and risky. Unknown selections remain gated by typed confirmation (<code>{TYPED_CONFIRMATION_PHRASE}</code>) and Protected Paths remain non-removable.</div>
        <label className="checkbox">
          <input type="checkbox" checked={draft.allowDeleteUnknown} onChange={(e) => setDraft({ ...draft, allowDeleteUnknown: e.target.checked })} />
          Allow deleting unknown folders (advanced)
        </label>
      </div>

      <div className="settings-section" aria-labelledby="custom-paths-heading">
        <h3 id="custom-paths-heading" className="section-title">Custom Paths</h3>
        <p className="muted">Custom Paths are validated (must exist, depth &ge; 3, not Protected, no parent fallback). Contents only; root folder is preserved. Selections require typed confirmation.</p>
        <div className="custom-path-add row">
          <div className="field field-flex custom-path-field">
            <label htmlFor="custom-path-input">Add Custom Path</label>
            <input id="custom-path-input" type="text" value={customPathInput} onChange={(e) => setCustomPathInput(e.target.value)} placeholder="C:\\Path\\To\\Folder" disabled={saving || validatingPath} />
          </div>
          <button className="btn" onClick={addCustomPath} disabled={saving || validatingPath || !customPathInput.trim()}>{validatingPath ? 'Validating…' : 'Add & Validate'}</button>
        </div>
        {draft.customPaths.length === 0 ? (
          <p className="muted">No Custom Paths configured.</p>
        ) : (
          <table className="table" aria-label="Custom Paths">
            <thead><tr><th>Path</th><th></th></tr></thead>
            <tbody>
              {draft.customPaths.map((p) => (
                <tr key={p}>
                  <td><code>{p}</code></td>
                  <td><button className="btn" onClick={() => removeCustomPath(p)} disabled={saving}>Remove</button></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <div className="row">
        <button className="btn primary" onClick={() => save()} disabled={saving}>{saving ? 'Saving…' : 'Save Settings'}</button>
        <button className="btn" onClick={() => settings && setDraft(settings)} disabled={saving}>Reset Changes</button>
      </div>
    </section>
  );
}
