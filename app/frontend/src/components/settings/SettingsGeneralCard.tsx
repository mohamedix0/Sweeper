import type { Settings } from '../../api/client';
import type { Theme } from './useSettingsController';

export function SettingsGeneralCard(props: {
  draft: Settings;
  setDraft: (s: Settings) => void;
  theme?: Theme;
  onThemeChange?: (theme: Theme) => void;
}) {
  const { draft, setDraft, theme, onThemeChange } = props;
  return (
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
            <p>Skip files changed more recently than this.</p>
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
  );
}
