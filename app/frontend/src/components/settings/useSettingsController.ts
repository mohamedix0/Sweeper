import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { formatContractError, type Settings, type SweeperClient } from '../../api/client';

export type Theme = 'dark' | 'light';
export type SettingsFocusTarget = 'allowDeleteUnknown' | 'enableTier3' | null;

const CUSTOM_PATH_PRESETS = ['C:\\Temp\\SweeperCleanup', 'C:\\Temp\\OldCache'];

function sameCustomPaths(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((p, i) => p === b[i]);
}

function fieldEqual(input: { a: Settings; b: Settings; key: keyof Settings }): boolean {
  if (input.key === 'customPaths') return sameCustomPaths(input.a.customPaths, input.b.customPaths);
  return input.a[input.key] === input.b[input.key];
}

function settingsEqual(a: Settings | null, b: Settings | null): boolean {
  if (a === b) return true;
  if (!a) return false;
  if (!b) return false;
  const keys: (keyof Settings)[] = ['enableTier1', 'enableTier2', 'enableTier3', 'enableElevationTargets', 'minAgeHours', 'allowDeleteUnknown', 'staleProjectDays', 'customPaths'];
  for (const key of keys) {
    if (!fieldEqual({ a, b, key })) return false;
  }
  return true;
}

function mergeField<K extends keyof Settings>(input: { saved: Settings; submitted: Settings; after: Settings; key: K }): Settings[K] {
  if (fieldEqual({ a: input.after, b: input.submitted, key: input.key })) return input.saved[input.key];
  return input.after[input.key];
}

function mergePreservingEdits(input: { saved: Settings; submitted: Settings; after: Settings }): Settings {
  return {
    enableTier1: mergeField({ ...input, key: 'enableTier1' }),
    enableTier2: mergeField({ ...input, key: 'enableTier2' }),
    enableTier3: mergeField({ ...input, key: 'enableTier3' }),
    enableElevationTargets: mergeField({ ...input, key: 'enableElevationTargets' }),
    minAgeHours: mergeField({ ...input, key: 'minAgeHours' }),
    allowDeleteUnknown: mergeField({ ...input, key: 'allowDeleteUnknown' }),
    customPaths: mergeField({ ...input, key: 'customPaths' }),
    staleProjectDays: mergeField({ ...input, key: 'staleProjectDays' }),
  };
}

function hasPrefill(value: string | null | undefined): value is string {
  if (value == null) return false;
  return value !== '';
}

function hasDuplicatePath(input: { paths: string[]; normalized: string }): boolean {
  return input.paths.some((p) => p.toLowerCase() === input.normalized.toLowerCase());
}

type ControllerArgs = {
  client: SweeperClient;
  settings: Settings | null;
  onSaved: (s: Settings) => void;
  focusTarget?: SettingsFocusTarget;
  onFocusConsumed?: () => void;
  prefillCustomPath?: string | null;
};

type UiState = {
  draft: Settings | null;
  setDraft: React.Dispatch<React.SetStateAction<Settings | null>>;
  loading: boolean;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  saving: boolean;
  setSaving: React.Dispatch<React.SetStateAction<boolean>>;
  error: string | null;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  info: string | null;
  setInfo: React.Dispatch<React.SetStateAction<string | null>>;
};

function useUiState(settings: Settings | null): UiState {
  const [draft, setDraft] = useState<Settings | null>(settings);
  const [loading, setLoading] = useState(!settings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  return { draft, setDraft, loading, setLoading, saving, setSaving, error, setError, info, setInfo };
}

function useControllerRefs(args: ControllerArgs) {
  const draftRef = useRef<Settings | null>(args.settings);
  const onSavedRef = useRef(args.onSaved);
  const onFocusConsumedRef = useRef(args.onFocusConsumed);
  onSavedRef.current = args.onSaved;
  onFocusConsumedRef.current = args.onFocusConsumed;
  return { draftRef, onSavedRef, onFocusConsumedRef };
}

function useSettingsSync(args: ControllerArgs, ui: UiState, refs: ReturnType<typeof useControllerRefs>) {
  const { settings, client } = args;
  const { draft, setDraft, setLoading, setError } = ui;
  useEffect(() => { refs.draftRef.current = draft; }, [refs, draft]);
  useEffect(() => {
    if (!settings) return;
    setDraft(settings);
    setLoading(false);
  }, [setDraft, setLoading, settings]);
  useEffect(() => {
    if (settings) return;
    let cancelled = false;
    void (async () => {
      try {
        const s = await client.getSettings();
        if (cancelled) return;
        setDraft(s);
        refs.onSavedRef.current(s);
      } catch (e) {
        if (!cancelled) setError(formatContractError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, refs, setDraft, setError, setLoading, settings]);
}

type FocusPrefillOptions = {
  args: ControllerArgs;
  refs: ReturnType<typeof useControllerRefs>;
  setCustomPathInput: (value: string) => void;
  allowUnknownRowRef: React.RefObject<HTMLDivElement>;
  setHighlight: (value: boolean) => void;
};

function applyPrefill(options: FocusPrefillOptions) {
  const path = options.args.prefillCustomPath;
  if (!hasPrefill(path)) return;
  options.setCustomPathInput(path);
  if (options.args.focusTarget === 'allowDeleteUnknown') return;
  options.refs.onFocusConsumedRef.current?.();
}

function focusAllowUnknownRow(options: FocusPrefillOptions): (() => void) | undefined {
  if (options.args.focusTarget !== 'allowDeleteUnknown') return undefined;
  const row = options.allowUnknownRowRef.current;
  if (!row) {
    options.refs.onFocusConsumedRef.current?.();
    return undefined;
  }
  row.scrollIntoView({ block: 'center', behavior: 'smooth' });
  const focusable = row.querySelector('input, button, [tabindex]') as HTMLElement | null;
  focusable?.focus();
  options.setHighlight(true);
  const timer = window.setTimeout(() => options.setHighlight(false), 1800);
  options.refs.onFocusConsumedRef.current?.();
  return () => window.clearTimeout(timer);
}

function useFocusAndPrefill(options: FocusPrefillOptions) {
  const prefillPath = options.args.prefillCustomPath;
  const focusTarget = options.args.focusTarget;
  useEffect(() => {
    applyPrefill(options);
  }, [focusTarget, options, prefillPath]);
  useEffect(() => focusAllowUnknownRow(options), [focusTarget, options]);
}

function useSaveHandler(client: SweeperClient, ui: UiState, refs: ReturnType<typeof useControllerRefs>) {
  const { setDraft, setError, setInfo, setSaving } = ui;
  // Single-flight guard: at most one `updateSettings` IPC call is in flight.
  // `toSave` is captured synchronously when the save is requested, not when the
  // queued task runs. Each save is chained behind the previous one, so requests
  // and responses are processed in submission order rather than reordered by
  // IPC. Edits that land after a save was submitted stay in the draft and are
  // preserved by `mergePreservingEdits` when that save's result is merged, so
  // the backend and UI converge on one consistent final state.
  const chainRef = useRef<Promise<void>>(Promise.resolve());
  return useCallback((next?: Settings): Promise<void> => {
    const toSave = next ?? refs.draftRef.current;
    if (!toSave) return Promise.resolve();
    refs.draftRef.current = toSave;
    setDraft(toSave);
    const run = chainRef.current.then(async () => {
      setSaving(true);
      setError(null);
      setInfo(null);
      try {
        const saved = await client.updateSettings(toSave);
        const after = refs.draftRef.current ?? saved;
        const merged = mergePreservingEdits({ saved, submitted: toSave, after });
        refs.draftRef.current = merged;
        setDraft(merged);
        refs.onSavedRef.current(merged);
        setInfo('Settings saved. Active sessions may be invalidated when scan-affecting settings change.');
      } catch (e) {
        setError(formatContractError(e));
      } finally {
        setSaving(false);
      }
    });
    // Keep the chain alive after a rejected task so later saves still run.
    chainRef.current = run.catch(() => {});
    return run;
  }, [client, refs, setDraft, setError, setInfo, setSaving]);
}

function useCustomPathActions(input: { client: SweeperClient; ui: UiState; refs: ReturnType<typeof useControllerRefs>; save: (next?: Settings) => Promise<void> }) {
  const [customPathInput, setCustomPathInput] = useState('');
  const [validatingPath, setValidatingPath] = useState(false);
  const [browseIndex, setBrowseIndex] = useState(0);
  const { client, ui, refs, save } = input;
  const { setError, setInfo } = ui;
  const addCustomPath = useCallback(async () => {
    const base = refs.draftRef.current;
    if (!base) return;
    const raw = customPathInput.trim();
    if (!raw) return;
    setValidatingPath(true);
    setError(null);
    setInfo(null);
    try {
      const verdict = await client.validateCustomPath(raw);
      if (!verdict.allowed) { setError('Custom Path rejected: ' + verdict.reason); return; }
      const current = refs.draftRef.current ?? base;
      const normalized = verdict.path || raw;
      if (hasDuplicatePath({ paths: current.customPaths, normalized })) { setError('Custom Path already added.'); return; }
      setCustomPathInput('');
      await save({ ...current, customPaths: [...current.customPaths, normalized] });
    } catch (e) {
      setError(formatContractError(e));
    } finally {
      setValidatingPath(false);
    }
  }, [client, customPathInput, refs, save, setError, setInfo]);
  const removeCustomPath = useCallback(async (path: string) => {
    const base = refs.draftRef.current;
    if (!base) return;
    await save({ ...base, customPaths: base.customPaths.filter((p) => p !== path) });
  }, [refs, save]);
  const applyCustomPathPreset = useCallback(() => {
    setCustomPathInput(CUSTOM_PATH_PRESETS[browseIndex % CUSTOM_PATH_PRESETS.length]);
    setBrowseIndex((i) => i + 1);
  }, [browseIndex]);
  return { customPathInput, setCustomPathInput, validatingPath, addCustomPath, removeCustomPath, applyCustomPathPreset };
}

function useRelaunchAction(client: SweeperClient) {
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchInfo, setRelaunchInfo] = useState<string | null>(null);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const runAsAdministrator = useCallback(async () => {
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
  }, [client]);
  return { relaunching, relaunchInfo, relaunchError, runAsAdministrator };
}

export function useSettingsController(args: ControllerArgs) {
  const ui = useUiState(args.settings);
  const [highlightAllowUnknown, setHighlightAllowUnknown] = useState(false);
  const allowUnknownRowRef = useRef<HTMLDivElement | null>(null);
  const refs = useControllerRefs(args);
  useSettingsSync(args, ui, refs);
  const save = useSaveHandler(args.client, ui, refs);
  const paths = useCustomPathActions({ client: args.client, ui, refs, save });
  useFocusAndPrefill({
    args,
    refs,
    setCustomPathInput: paths.setCustomPathInput,
    allowUnknownRowRef,
    setHighlight: setHighlightAllowUnknown,
  });
  const relaunch = useRelaunchAction(args.client);
  const isDirty = useMemo(() => !settingsEqual(ui.draft, args.settings), [ui.draft, args.settings]);
  const deepDisabled = !!ui.draft && !ui.draft.enableTier3;
  const { setDraft } = ui;
  const resetDraft = useCallback(() => { if (args.settings) setDraft(args.settings); }, [args.settings, setDraft]);
  return {
    draft: ui.draft, setDraft: ui.setDraft, loading: ui.loading, saving: ui.saving, error: ui.error, info: ui.info,
    customPathInput: paths.customPathInput, setCustomPathInput: paths.setCustomPathInput, validatingPath: paths.validatingPath,
    relaunching: relaunch.relaunching, relaunchInfo: relaunch.relaunchInfo, relaunchError: relaunch.relaunchError,
    highlightAllowUnknown, allowUnknownRowRef, isDirty, deepDisabled,
    save, addCustomPath: paths.addCustomPath, removeCustomPath: paths.removeCustomPath, runAsAdministrator: relaunch.runAsAdministrator, applyCustomPathPreset: paths.applyCustomPathPreset, resetDraft,
  };
}