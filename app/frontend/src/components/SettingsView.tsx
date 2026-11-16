import type { Settings, SweeperClient } from '../api/client';
import { SettingsFooterBar } from './settings/SettingsFooterBar';
import { SettingsGeneralCard } from './settings/SettingsGeneralCard';
import { SettingsCustomPathsCard } from './settings/SettingsCustomPathsCard';
import { SettingsDeepCleanupCard } from './settings/SettingsDeepCleanupCard';
import { SettingsSystemCachesCard } from './settings/SettingsSystemCachesCard';
import { useSettingsController, type SettingsFocusTarget, type Theme } from './settings/useSettingsController';

export function SettingsView({
  client,
  settings,
  elevationStatus,
  elevationError,
  onSaved,
  theme,
  onThemeChange,
  focusTarget,
  onFocusConsumed,
  prefillCustomPath,
}: {
  client: SweeperClient;
  settings: Settings | null;
  elevationStatus: { isElevated: boolean };
  elevationError: string | null;
  onSaved: (s: Settings) => void;
  theme?: Theme;
  onThemeChange?: (theme: Theme) => void;
  focusTarget?: SettingsFocusTarget;
  onFocusConsumed?: () => void;
  prefillCustomPath?: string | null;
}) {
  const c = useSettingsController({ client, settings, onSaved, focusTarget, onFocusConsumed, prefillCustomPath });
  if (c.loading || !c.draft) {
    return (
      <div className="view-stack">
        <header className="page-header">
          <div className="page-header-left"><h1 className="page-title" id="settings-heading">Settings</h1></div>
        </header>
        {c.error ? <div className="notice error" role="alert">{c.error}</div> : <p className="muted">Loading settings…</p>}
      </div>
    );
  }
  return (
    <div className="view-stack settings-view-stack">
      <header className="page-header" aria-label="Settings header">
        <div className="page-header-left">
          <h1 className="page-title" id="settings-heading">Settings</h1>
          {!c.isDirty && <span className="save-status-plain muted" aria-label="All changes saved">All changes saved</span>}
        </div>
      </header>
      {elevationError && <div className="notice error" role="alert">Elevation status unavailable: {elevationError}</div>}
      {c.error && <div className="notice error" role="alert">{c.error}</div>}
      {c.info && <div className="notice info" role="status">{c.info}</div>}
      {c.relaunchInfo && <div className="notice info" role="status">{c.relaunchInfo}</div>}
      {c.relaunchError && <div className="notice error" role="alert">{c.relaunchError}</div>}
      <div className="settings-page-wrap">
        <div className="settings-form-wrap settings-v13">
          <div className="settings-columns">
            <div className="settings-col">
              <SettingsGeneralCard draft={c.draft} setDraft={c.setDraft} theme={theme} onThemeChange={onThemeChange} />
              <SettingsCustomPathsCard draft={c.draft} setDraft={c.setDraft} saving={c.saving} validatingPath={c.validatingPath} customPathInput={c.customPathInput} setCustomPathInput={c.setCustomPathInput} onApplyPreset={c.applyCustomPathPreset} onAdd={c.addCustomPath} onRemove={c.removeCustomPath} />
            </div>
            <div className="settings-col">
              <SettingsDeepCleanupCard draft={c.draft} setDraft={c.setDraft} deepDisabled={c.deepDisabled} highlightAllowUnknown={c.highlightAllowUnknown} allowUnknownRowRef={c.allowUnknownRowRef} />
              <SettingsSystemCachesCard draft={c.draft} setDraft={c.setDraft} deepDisabled={c.deepDisabled} elevationStatus={elevationStatus} relaunching={c.relaunching} onRunAsAdmin={c.runAsAdministrator} />
            </div>
          </div>
          <SettingsFooterBar isDirty={c.isDirty} saving={c.saving} onReset={c.resetDraft} onSave={() => c.save()} />
        </div>
      </div>
    </div>
  );
}
