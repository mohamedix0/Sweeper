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

function mergeField<K extends keyof Settings>(args: { saved: Settings; submitted: Settings; after: Settings; key: K }): Settings[K] {
  if (settingsEqualField(args.after, args.submitted, args.key)) return args.saved[args.key];
  return args.after[args.key];
}

function settingsEqualField(a: Settings, b: Settings, key: keyof Settings): boolean {
  if (key !== 'customPaths') return a[key] === b[key];
  return sameCustomPaths(a.customPaths, b.customPaths);
}

function mergePreservingEdits(args: { saved: Settings; submitted: Settings; after: Settings }): Settings {
  return {
    enableTier1: mergeField({ ...args, key: 'enableTier1' }),
    enableTier2: mergeField({ ...args, key: 'enableTier2' }),
    enableTier3: mergeField({ ...args, key: 'enableTier3' }),
    enableElevationTargets: mergeField({ ...args, key: 'enableElevationTargets' }),
    minAgeHours: mergeField({ ...args, key: 'minAgeHours' }),
    allowDeleteUnknown: mergeField({ ...args, key: 'allowDeleteUnknown' }),
    customPaths: mergeField({ ...args, key: 'customPaths' }),
    staleProjectDays: mergeField({ ...args, key: 'staleProjectDays' }),
  };
}

function hasPrefill(value: string | null | undefined): value is string {
  if (value == null) return false;
  return value !== '';
}

function consumePrefillFocus(args: { focusTarget?: SettingsFocusTarget; onFocusConsumed?: () => void }) {
  if (args.focusTarget === 'allowDeleteUnknown') return;
  args.onFocusConsumed?.();
}

function focusAllowUnknownRow(row: HTMLDivElement | null, setHighlight: (v: boolean) => void) {
  if (!row) return;
  row.scrollIntoView({ block: 'center', behavior: 'smooth' });
  const focusable = row.querySelector('input, button, [tabindex]') as HTMLElement | null;
  focusable?.focus();
  setHighlight(true);
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
  const draftRef = useRef<Settings | null>(settings);
  const onSavedRef = useRef(onSaved);
  const onFocusConsumedRef = useRef(onFocusConsumed);
  onSavedRef.current = onSaved;
  onFocusConsumedRef.current = onFocusConsumed;

  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  useEffect(() => {
    if (!hasPrefill(prefillCustomPath)) return;
    setCustomPathInput(prefillCustomPath);
    consumePrefillFocus({ focusTarget, onFocusConsumed: onFocusConsumedRef.current });
  }, [prefillCustomPath, focusTarget]);

  useEffect(() => {
    if (focusTarget !== 'allowDeleteUnknown') return;
    focusAllowUnknownRow(allowUnknownRowRef.current, setHighlightAllowUnknown);
    const t = window.setTimeout(() => setHighlightAllowUnknown(false), 1800);
    onFocusConsumedRef.current?.();
    return () => window.clearTimeout(t);
  }, [focusTarget]);

  useEffect(() => {
    if (!settings) return;
    setDraft(settings);
    setLoading(false);
  }, [settings]);

  useEffect(() => {
    if (settings) return;
    let cancelled = false;
    (async () => {
      try {
        const s = await client.getSettings();
        if (cancelled) return;
        setDraft(s);
        onSavedRef.current(s);
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, settings]);

  const isDirty = useMemo(() => !settingsEqual(draft, settings), [draft, settings]);
  const deepDisabled = !!draft && !draft.enableTier3;

  async function save(next?: Settings) {
    const toSave = next ?? draftRef.current;
    if (!toSave) return;
    setSaving(true);
    setError(null);
    setInfo(null);
    draftRef.current = toSave;
    setDraft(toSave);
    try {
      const saved = await client.updateSettings(toSave);
      const after = draftRef.current ?? saved;
      const merged = mergePreservingEdits({ saved, submitted: toSave, after });
      draftRef.current = merged;
      setDraft(merged);
      onSavedRef.current(merged);
      setInfo('Settings saved. Active sessions may be invalidated when scan-affecting settings change.');
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setSaving(false);
    }
  }

  function hasDuplicatePath(args: { paths: string[]; normalized: string }): boolean {
    return args.paths.some((p) => p.toLowerCase() === args.normalized.toLowerCase());
  }

  async function addCustomPath() {
    const base = draftRef.current;
    if (!base) return;
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
      const current = draftRef.current ?? base;
      const normalized = verdict.path || raw;
      if (hasDuplicatePath({ paths: current.customPaths, normalized })) {
        setError('Custom Path already added.');
        return;
      }
      const next = { ...current, customPaths: [...current.customPaths, normalized] };
      setCustomPathInput('');
      await save(next);
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setValidatingPath(false);
    }
  }

  async function removeCustomPath(path: string) {
    const base = draftRef.current;
    if (!base) return;
    const next = { ...base, customPaths: base.customPaths.filter((p) => p !== path) };
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
