import type { Settings } from '../../api/client';

export function SettingsCustomPathsCard(props: {
  draft: Settings;
  setDraft: (s: Settings) => void;
  saving: boolean;
  validatingPath: boolean;
  customPathInput: string;
  setCustomPathInput: (v: string) => void;
  onApplyPreset: () => void;
  onAdd: () => void;
  onRemove: (path: string) => void;
}) {
  const { draft, setDraft, saving, validatingPath, customPathInput, setCustomPathInput, onApplyPreset, onAdd, onRemove } = props;
  return (
    <section className="settings-group" aria-labelledby="custom-paths-heading">
      <h2 id="custom-paths-heading" className="settings-group-title">Custom Paths</h2>
      <div className="settings-group-card">
        <div className="settings-list-row settings-list-row-stack">
          <div className="settings-row-copy">
            <h3>Add Custom Path</h3>
            <p>Add extra folders here to scan alongside built-in targets. Protected system directories are always excluded.</p>
          </div>
          <div className="custom-path-input-row">
            <label htmlFor="custom-path-input" className="sr-only">Add Custom Path</label>
            <input id="custom-path-input" type="text" value={customPathInput} onChange={(e) => setCustomPathInput(e.target.value)} placeholder="C:\Path\To\Folder" disabled={saving || validatingPath} />
            <button type="button" className="btn" onClick={onApplyPreset} disabled={saving || validatingPath} aria-label="Use example custom path preset">Use Preset</button>
            <button className="btn" onClick={onAdd} disabled={saving || validatingPath || !customPathInput.trim()} aria-label="Add Folder">{validatingPath ? 'Validating…' : 'Add Folder'}</button>
          </div>
        </div>
        <div className="settings-list-row settings-custom-optin-row">
          <div className="settings-inline-note muted settings-custom-optin-note">
            <label className="checkbox inline-plain-checkbox">
              <input type="checkbox" aria-label="Allow cleanup of custom paths" checked={draft.allowDeleteUnknown} onChange={(e) => setDraft({ ...draft, allowDeleteUnknown: e.target.checked })} />
              <span>Allow cleanup of custom paths (requires DELETE confirmation)</span>
            </label>
          </div>
        </div>
        {draft.customPaths.length === 0 ? (
          <div className="settings-inline-note muted">No custom folders added yet. Any unverified custom targets found during a scan remain read-only until added here and enabled for cleanup.</div>
        ) : (
          <div className="custom-path-list">
            {draft.customPaths.map((p) => (
              <div key={p} className="custom-path-row">
                <code className="custom-path-value">{p}</code>
                <button className="btn small" onClick={() => onRemove(p)} disabled={saving}>Remove</button>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
