import type { Settings } from '../../api/client';

export function SettingsFooterBar(props: {
  isDirty: boolean;
  saving: boolean;
  inputsDisabled?: boolean;
  onReset: () => void;
  onSave: () => void;
}) {
  if (!props.isDirty) return null;
  const locked = props.saving || !!props.inputsDisabled;
  return (
    <div className="settings-form-actions" aria-label="Settings actions">
      <button className="btn" onClick={props.onReset} disabled={locked}>Reset Changes</button>
      <button className="btn primary" onClick={props.onSave} disabled={locked}>{props.saving ? 'Saving…' : 'Save Settings'}</button>
    </div>
  );
}
