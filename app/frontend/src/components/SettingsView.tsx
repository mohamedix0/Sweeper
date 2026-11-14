import { useEffect, useMemo, useState } from 'react';
import { formatContractError, type Settings, type SweeperClient } from '../api/client';

type Theme = 'dark' | 'light';

const BROWSE_SUGGESTIONS = ['C:\\Users\\Alice\\CustomCleanup', 'C:\\Temp\\Cleanup', 'D:\\Scratch'];

function settingsEqual(a: Settings | null, b: Settings | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.enableTier1 === b.enableTier1
    && a.enableTier2 === b.enableTier2
    && a.enableTier3 === b.enableTier3
    && a.enableElevationTargets === b.enableElevationTargets
    && a.minAgeHours === b.minAgeHours
    && a.allowDeleteUnknown === b.allowDeleteUnknown
    && a.staleProjectDays === b.staleProjectDays
    && a.customPaths.length === b.customPaths.length
    && a.customPaths.every((p, i) => p === b.customPaths[i]);
}

export function SettingsView({
  client,
  settings,
  elevationStatus,
  elevationError,
  onSaved,
  theme,
  onThemeChange,
}: {
  client: SweeperClient;
  settings: Settings | null;
  elevationStatus: { isElevated: boolean };
  elevationError: string | null;
  onSaved: (s: Settings) => void;
  theme?: Theme;
  onThemeChange?: (theme: Theme) => void;
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
  const [browseIndex, setBrowseIndex] = useState(0);

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

  const isDirty = useMemo(() => !settingsEqual(draft, settings), [draft, settings]);

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
      if (res.launched) setRelaunchInfo('Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.');
      else setRelaunchError('Administrator launch was not started.');
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  }

  function browseCustomPath() {
    const suggestion = BROWSE_SUGGESTIONS[browseIndex % BROWSE_SUGGESTIONS.length];
    setCustomPathInput(suggestion);
    setBrowseIndex((i) => i + 1);
  }

  if (loading || !draft) {
    return (
      <div className="view-stack">
        <header className="page-header">
          <div className="page-header-left"><h1 className="page-title" id="settings-heading">Settings</h1></div>
        </header>
        {error ? <div className="notice error" role="alert">{error}</div> : <p className="muted">Loading settings…</p>}
      </div>
    );
  }

  const deepDisabled = !draft.enableTier3;

  return (
    <div className="view-stack settings-view-stack">
      <header className="page-header" aria-label="Settings header">
        <div className="page-header-left">
          <h1 className="page-title" id="settings-heading">Settings</h1>
          {!isDirty && <span className="save-status-plain muted" aria-label="All changes saved">All changes saved</span>}
        </div>
      </header>

      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}
      {relaunchInfo && <div className="notice info" role="status">{relaunchInfo}</div>}
      {relaunchError && <div className="notice error" role="alert">{relaunchError}</div>}

      <div className="settings-page-wrap">
        <div className="settings-form-wrap settings-fluent-list">
          <section className="settings-group" aria-labelledby="general-settings-heading">
            <h2 id="general-settings-heading" className="settings-group-title">General</h2>
            <div className="settings-group-card">
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Standard Temporary Files</h3>
                  <p>Clean system and app temporary files that rebuild automatically.</p>
                </div>
                <div className="settings-row-control">
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Enable Tier 1" checked={draft.enableTier1} onChange={(e) => setDraft({ ...draft, enableTier1: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Developer &amp; Package Caches</h3>
                  <p>Clean package and build caches that can be downloaded or rebuilt again.</p>
                </div>
                <div className="settings-row-control">
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Enable Tier 2" checked={draft.enableTier2} onChange={(e) => setDraft({ ...draft, enableTier2: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3><label htmlFor="min-age-hours">Minimum file age</label></h3>
                  <p>Skip files changed more recently than this to avoid removing active data.</p>
                  {draft.minAgeHours < 24 && <span className="inline-warn-text" role="note">Warning: below 24 hours may include very recent files.</span>}
                </div>
                <div className="settings-row-control numeric-control">
                  <input id="min-age-hours" className="compact-number" type="number" min={0} value={draft.minAgeHours} onChange={(e) => setDraft({ ...draft, minAgeHours: Number(e.target.value || 0) })} />
                  <span className="unit-suffix">hours</span>
                </div>
              </div>
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Appearance</h3>
                  <p>Choose how Sweeper looks on this device.</p>
                </div>
                <div className="settings-row-control">
                  <div className="theme-toggle" role="group" aria-label="Appearance">
                    <button type="button" className={theme === 'dark' ? 'theme-segment active' : 'theme-segment'} aria-pressed={theme === 'dark'} onClick={() => onThemeChange?.('dark')} disabled={!onThemeChange}>Dark</button>
                    <button type="button" className={theme === 'light' ? 'theme-segment active' : 'theme-segment'} aria-pressed={theme === 'light'} onClick={() => onThemeChange?.('light')} disabled={!onThemeChange}>Light</button>
                  </div>
                </div>
              </div>
            </div>
          </section>

          <section className="settings-group" aria-labelledby="tier3-settings-heading">
            <h2 id="tier3-settings-heading" className="settings-group-title">Deep Cleanup &amp; Inactive Projects</h2>
            <div className="settings-group-card">
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Enable Deep Cleanup Categories</h3>
                  <p>Opt in to higher-risk cleanup categories. Protected Paths are never removable.</p>
                </div>
                <div className="settings-row-control">
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Enable deep cleanup categories" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className={`settings-list-row ${deepDisabled ? 'is-disabled-row' : ''}`}>
                <div className="settings-row-copy">
                  <h3>Inactive Node.js dependencies (node_modules/)</h3>
                  <p>Highlight inactive project caches for cleanup when Deep Cleanup is enabled.</p>
                </div>
                <div className="settings-row-control settings-row-control-cluster">
                  <span className="badge risk-Safe">Safe · Rebuildable</span>
                  <label className="inline-threshold" htmlFor="stale-project-days-node">
                    <input id="stale-project-days-node" className="compact-number" type="number" min={1} value={draft.staleProjectDays} onChange={(e) => setDraft({ ...draft, staleProjectDays: Number(e.target.value || 30) })} disabled={deepDisabled} aria-label="Inactive Node.js dependencies age" />
                    <span className="unit-suffix">days</span>
                  </label>
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Inactive Node.js dependencies toggle" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className={`settings-list-row ${deepDisabled ? 'is-disabled-row' : ''}`}>
                <div className="settings-row-copy">
                  <h3>Inactive Rust build artifacts (target/)</h3>
                  <p>Highlight inactive Rust build caches older than the configured threshold.</p>
                </div>
                <div className="settings-row-control settings-row-control-cluster">
                  <span className="badge risk-Safe">Safe · Rebuildable</span>
                  <label className="inline-threshold" htmlFor="stale-project-days">
                    <input id="stale-project-days" className="compact-number" type="number" min={1} value={draft.staleProjectDays} onChange={(e) => setDraft({ ...draft, staleProjectDays: Number(e.target.value || 30) })} disabled={deepDisabled} aria-label="Inactive project cache age" />
                    <span className="unit-suffix">days</span>
                  </label>
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Inactive Rust build artifacts toggle" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className={`settings-list-row ${deepDisabled ? 'is-disabled-row' : ''}`}>
                <div className="settings-row-copy">
                  <h3>Empty Windows Recycle Bin</h3>
                  <p>Permanently empties the Recycle Bin when Deep Cleanup is enabled.</p>
                </div>
                <div className="settings-row-control settings-row-control-cluster">
                  <span className="badge risk-Caution">Caution · Permanent</span>
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Empty Windows Recycle Bin toggle" checked={draft.enableTier3} onChange={(e) => setDraft({ ...draft, enableTier3: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Allow deleting unknown folders</h3>
                  <p>Advanced. Unknown folders always require explicit typed confirmation before deletion.</p>
                </div>
                <div className="settings-row-control settings-row-control-cluster">
                  <span className="badge risk-Unknown">Requires DELETE confirm</span>
                  <label className="checkbox tier-toggle">
                    <input type="checkbox" aria-label="Allow deleting unknown folders (advanced)" checked={draft.allowDeleteUnknown} onChange={(e) => setDraft({ ...draft, allowDeleteUnknown: e.target.checked })} />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              <div className="settings-list-row settings-list-row-info">
                <div className="settings-row-copy">
                  <h3>WSL &amp; Docker virtual disks (.vhdx)</h3>
                  <p>Large virtual disks are shown for context and cannot be deleted by Sweeper.</p>
                </div>
                <div className="settings-row-control">
                  <span className="badge status-readonly">Read-only</span>
                </div>
              </div>
            </div>
          </section>

          <section className="settings-group" aria-labelledby="system-wide-caches-heading">
            <h2 id="system-wide-caches-heading" className="settings-group-title">System-Wide Windows Caches</h2>
            <div className="settings-group-card">
              <div className="settings-list-row">
                <div className="settings-row-copy">
                  <h3>Allow system-wide Windows caches</h3>
                  <p>Requires administrator elevation and Deep Cleanup opt-in.</p>
                </div>
                <div className="settings-row-control settings-row-control-cluster">
                  {!elevationStatus.isElevated && (
                    <button className="btn small" onClick={runAsAdministrator} disabled={relaunching} aria-label="Run as administrator">Elevate &amp; Enable</button>
                  )}
                  <label className="checkbox tier-toggle">
                    <input
                      type="checkbox"
                      aria-label="Allow system-wide Windows caches (Admin)"
                      checked={draft.enableElevationTargets}
                      onChange={(e) => setDraft({ ...draft, enableElevationTargets: e.target.checked })}
                      disabled={!elevationStatus.isElevated || !draft.enableTier3}
                    />
                    <span className="tier-switch" aria-hidden="true" />
                  </label>
                </div>
              </div>
              {!elevationStatus.isElevated && (
                <div className="settings-inline-note muted">Run as administrator to unlock system-wide caches. Deep Cleanup must also be enabled.</div>
              )}
              {elevationStatus.isElevated && !draft.enableTier3 && (
                <div className="settings-inline-note muted">Enable Deep Cleanup categories to use system-wide Windows caches.</div>
              )}
            </div>
          </section>

          <section className="settings-group" aria-labelledby="custom-paths-heading">
            <h2 id="custom-paths-heading" className="settings-group-title">Custom Paths</h2>
            <div className="settings-group-card">
              <div className="settings-list-row settings-list-row-stack">
                <div className="settings-row-copy">
                  <h3>Add Custom Path</h3>
                  <p>Custom folders are scanned for eligible temporary items. Protected system directories are always excluded.</p>
                </div>
                <div className="custom-path-input-row">
                  <label htmlFor="custom-path-input" className="sr-only">Add Custom Path</label>
                  <input id="custom-path-input" type="text" value={customPathInput} onChange={(e) => setCustomPathInput(e.target.value)} placeholder="C:\Path\To\Folder" disabled={saving || validatingPath} />
                  <button type="button" className="btn" onClick={browseCustomPath} disabled={saving || validatingPath} aria-label="Browse for custom path">Browse...</button>
                  <button className="btn" onClick={addCustomPath} disabled={saving || validatingPath || !customPathInput.trim()}>{validatingPath ? 'Validating…' : 'Add Folder'}</button>
                </div>
              </div>
              {draft.customPaths.length === 0 ? (
                <div className="settings-inline-note muted">No Custom Paths configured.</div>
              ) : (
                <div className="custom-path-list">
                  {draft.customPaths.map((p) => (
                    <div key={p} className="custom-path-row">
                      <code className="custom-path-value">{p}</code>
                      <button className="btn small" onClick={() => removeCustomPath(p)} disabled={saving}>Remove</button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </section>

          {isDirty && (
            <div className="settings-form-actions" aria-label="Settings actions">
              <button className="btn" onClick={() => settings && setDraft(settings)} disabled={saving}>Reset Changes</button>
              <button className="btn primary" onClick={() => save()} disabled={saving}>{saving ? 'Saving…' : 'Save Settings'}</button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
