import type { Settings } from '../../api/client';

export function SettingsFooterBar(props: {
  isDirty: boolean;
  saving: boolean;
  onReset: () => void;
  onSave: () => void;
}) {
  if (!props.isDirty) return null;
  return (
    <div className="settings-form-actions" aria-label="Settings actions">
      <button className="btn" onClick={props.onReset} disabled={props.saving}>Reset Changes</button>
      <button className="btn primary" onClick={props.onSave} disabled={props.saving}>{props.saving ? 'Saving…' : 'Save Settings'}</button>
    </div>
  );
}
