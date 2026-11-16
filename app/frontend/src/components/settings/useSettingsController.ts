import { useEffect, useMemo, useRef, useState } from 'react';
import { formatContractError, type Settings, type SweeperClient } from '../../api/client';

export type Theme = 'dark' | 'light';
export type SettingsFocusTarget = 'allowDeleteUnknown' | 'enableTier3' | null;

const CUSTOM_PATH_PRESETS = ['C:\\Temp\\SweeperCleanup', 'C:\\Temp\\OldCache'];

function sameCustomPaths(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((p, i) => p === b[i]);
}

function settingsEqual(a: Settings | null, b: Settings | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.enableTier1 !== b.enableTier1) return false;
  if (a.enableTier2 !== b.enableTier2) return false;
  if (a.enableTier3 !== b.enableTier3) return false;
  if (a.enableElevationTargets !== b.enableElevationTargets) return false;
  if (a.minAgeHours !== b.minAgeHours) return false;
  if (a.allowDeleteUnknown !== b.allowDeleteUnknown) return false;
  if (a.staleProjectDays !== b.staleProjectDays) return false;
  return sameCustomPaths(a.customPaths, b.customPaths);
}

export function useSettingsController(args: {
  client: SweeperClient;
  settings: Settings | null;
  onSaved: (s: Settings) => void;
  focusTarget?: SettingsFocusTarget;
  onFocusConsumed?: () => void;
  prefillCustomPath?: string | null;
}) {
  const { client, settings, onSaved, focusTarget, onFocusConsumed, prefillCustomPath } = args;
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
  const [highlightAllowUnknown, setHighlightAllowUnknown] = useState(false);
  const allowUnknownRowRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (prefillCustomPath == null || prefillCustomPath === '') return;
    setCustomPathInput(prefillCustomPath);
    if (focusTarget !== 'allowDeleteUnknown') onFocusConsumed?.();
  }, [prefillCustomPath, focusTarget, onFocusConsumed]);

  useEffect(() => {
    if (focusTarget !== 'allowDeleteUnknown') return;
    const row = allowUnknownRowRef.current;
    if (!row) return;
    row.scrollIntoView({ block: 'center', behavior: 'smooth' });
    const focusable = row.querySelector('input, button, [tabindex]') as HTMLElement | null;
    focusable?.focus();
    setHighlightAllowUnknown(true);
    const t = window.setTimeout(() => setHighlightAllowUnknown(false), 1800);
    onFocusConsumed?.();
    return () => window.clearTimeout(t);
  }, [focusTarget, onFocusConsumed]);

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
        if (cancelled) return;
        setDraft(s);
        onSaved(s);
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, settings, onSaved]);

  const isDirty = useMemo(() => !settingsEqual(draft, settings), [draft, settings]);
  const deepDisabled = !!draft && !draft.enableTier3;

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

  function applyCustomPathPreset() {
    const suggestion = CUSTOM_PATH_PRESETS[browseIndex % CUSTOM_PATH_PRESETS.length];
    setCustomPathInput(suggestion);
    setBrowseIndex((i) => i + 1);
  }

  function resetDraft() {
    if (settings) setDraft(settings);
  }

  return {
    draft, setDraft, loading, saving, error, info,
    customPathInput, setCustomPathInput, validatingPath,
    relaunching, relaunchInfo, relaunchError,
    highlightAllowUnknown, allowUnknownRowRef,
    isDirty, deepDisabled,
    save, addCustomPath, removeCustomPath, runAsAdministrator, applyCustomPathPreset, resetDraft,
  };
}
