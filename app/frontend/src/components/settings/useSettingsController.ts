import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
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

// Reconcile parent feedback skipped while saves were pending. `lastSubmitted`
// is the exact value the controller last submitted; `draft` may additionally
// hold a genuine user edit made after that submission. Fields still equal to
// the submission adopt the parent value, while genuine edits and newer
// submissions are preserved by mergePreservingEdits (the 02c5cb3 fix).
function reconcileDeferredFeedback(input: {
  settings: Settings;
  lastSubmitted: Settings | null;
  draft: Settings | null;
}): Settings {
  const submitted = input.lastSubmitted ?? input.settings;
  const after = input.draft ?? input.settings;
  return mergePreservingEdits({ saved: input.settings, submitted, after });
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
  // The exact value the backend last acknowledged, kept separate from the
  // editable draft. A save that lands while a newer edit is in the draft only
  // acknowledges what it actually persisted, so Elevate still flushes the
  // newer draft instead of mistaking it for saved (#92).
  acknowledged: Settings | null;
  setAcknowledged: React.Dispatch<React.SetStateAction<Settings | null>>;
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
  const [acknowledged, setAcknowledged] = useState<Settings | null>(settings);
  const [loading, setLoading] = useState(!settings);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  return { draft, setDraft, acknowledged, setAcknowledged, loading, setLoading, saving, setSaving, error, setError, info, setInfo };
}

function useControllerRefs(args: ControllerArgs) {
  const draftRef = useRef<Settings | null>(args.settings);
  // Backend-acknowledged value, read at flush time so a stale render cannot
  // decide that a draft matching an in-flight save is already persisted.
  const acknowledgedRef = useRef<Settings | null>(args.settings);
  const onSavedRef = useRef(args.onSaved);
  const onFocusConsumedRef = useRef(args.onFocusConsumed);
  // Baseline for detecting genuine user edits when reconciling deferred parent
  // feedback. It starts at the value this controller mounted with and is bumped
  // to exactly what each save submits, so a later draft change is recognisable
  // as a real edit rather than a previously skipped (stale) parent value.
  const lastSubmittedRef = useRef<Settings | null>(args.settings);
  // Tracks the queue-drain version already observed by the sync effect so it
  // can tell a drain-triggered rerun from an ordinary parent settings update.
  const seenQueueVersionRef = useRef<number>(getSaveQueueVersion());
  // Parent value last adopted into the draft. Comparing an incoming `settings`
  // against it tells whether feedback skipped while saves were pending is real
  // (a changed parent value) rather than an incidental effect rerun.
  const lastAppliedParentRef = useRef<Settings | null>(args.settings);
  // Set when a changed parent value is skipped because saves are still pending;
  // cleared once that feedback is replayed or applied directly. A queue drain
  // with no such feedback must leave a rejected local submission untouched.
  const deferredFeedbackRef = useRef<boolean>(false);
  onSavedRef.current = args.onSaved;
  onFocusConsumedRef.current = args.onFocusConsumed;
  // Keep the bundle stable across renders: all members are ref objects, so the
  // sync effect can safely depend on `refs` without rerunning on every draft
  // edit and overwriting unsaved user changes with parent feedback.
  return useMemo(
    () => ({ draftRef, acknowledgedRef, onSavedRef, onFocusConsumedRef, lastSubmittedRef, seenQueueVersionRef, lastAppliedParentRef, deferredFeedbackRef }),
    [draftRef, acknowledgedRef, onSavedRef, onFocusConsumedRef, lastSubmittedRef, seenQueueVersionRef, lastAppliedParentRef, deferredFeedbackRef],
  );
}

// A queue-drain rerun replays deferred parent feedback through an edit-preserving
// merge only when a changed parent value was actually skipped while saves were
// pending. With no real feedback to apply the draft is left alone, so a rejected
// submission is not silently replaced by the unchanged parent value.
function shouldReplayDeferredFeedback(input: { drained: boolean; hasDeferredFeedback: boolean }): boolean {
  return input.drained && input.hasDeferredFeedback;
}

// Record that a changed parent value arrived while saves were pending, so the
// eventual queue drain knows there is real feedback to replay. An unchanged
// parent value carries no deferred feedback and must not trigger a replay.
function recordDeferredFeedback(options: {
  settings: Settings;
  refs: ReturnType<typeof useControllerRefs>;
}): void {
  if (settingsEqual(options.refs.lastAppliedParentRef.current, options.settings)) return;
  options.refs.deferredFeedbackRef.current = true;
}

// Replay parent feedback that was skipped while saves were pending. The merge
// keeps genuine user edits and newer submissions (see reconcileDeferredFeedback).
function applyDeferredFeedback(options: {
  settings: Settings;
  refs: ReturnType<typeof useControllerRefs>;
  setDraft: React.Dispatch<React.SetStateAction<Settings | null>>;
}): void {
  options.setDraft(reconcileDeferredFeedback({
    settings: options.settings,
    lastSubmitted: options.refs.lastSubmittedRef.current,
    draft: options.refs.draftRef.current,
  }));
  options.refs.lastAppliedParentRef.current = options.settings;
  options.refs.deferredFeedbackRef.current = false;
}

// Effect body: mirror the draft into the ref bundle so save handlers and the
// reconciliation merge always read the latest local value.
function mirrorDraftIntoRef(options: {
  refs: ReturnType<typeof useControllerRefs>;
  draft: Settings | null;
}): void {
  options.refs.draftRef.current = options.draft;
}

// Adopt an ordinary parent update outright. A queue drain with no real deferred
// feedback (e.g. a rejected save whose parent value never changed) leaves the
// local draft untouched so the rejected submission is not silently replaced.
function adoptParentFeedback(options: {
  settings: Settings;
  refs: ReturnType<typeof useControllerRefs>;
  setDraft: React.Dispatch<React.SetStateAction<Settings | null>>;
  drained: boolean;
}): void {
  if (options.drained) return;
  options.refs.lastAppliedParentRef.current = options.settings;
  options.refs.deferredFeedbackRef.current = false;
  options.setDraft(options.settings);
}

// Effect body: adopt parent feedback when no save is outstanding. A queue drain
// (success OR failure) replays skipped feedback via a merge; an ordinary parent
// update replaces the draft outright.
function syncParentFeedback(options: {
  settings: Settings | null;
  refs: ReturnType<typeof useControllerRefs>;
  queueVersion: number;
  setDraft: React.Dispatch<React.SetStateAction<Settings | null>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
}): void {
  const { settings, refs, queueVersion, setDraft, setLoading } = options;
  // A changed queue version means the save queue just became empty (on success
  // OR failure). Record it before the early returns so a drain that coincides
  // with a null `settings` is not mistaken for a later drain.
  const drained = refs.seenQueueVersionRef.current !== queueVersion;
  refs.seenQueueVersionRef.current = queueVersion;
  if (!settings) return;
  setLoading(false);
  // While a save is still outstanding, parent feedback is from an earlier
  // queued save and must not overwrite the draft holding the latest submitted
  // value. It is applied by the save handler's own merge instead. Record real
  // (changed) feedback so a later drain can replay it.
  if (pendingSaves > 0) {
    recordDeferredFeedback({ settings, refs });
    return;
  }
  if (shouldReplayDeferredFeedback({ drained, hasDeferredFeedback: refs.deferredFeedbackRef.current })) {
    applyDeferredFeedback({ settings, refs, setDraft });
    return;
  }
  adoptParentFeedback({ settings, refs, setDraft, drained });
}

// Effect body: load settings from the client on mount when the parent has not
// supplied them yet. Returns the cancellation cleanup for the effect.
function loadSettingsWhenMissing(options: {
  client: SweeperClient;
  refs: ReturnType<typeof useControllerRefs>;
  setDraft: React.Dispatch<React.SetStateAction<Settings | null>>;
  setAcknowledged: React.Dispatch<React.SetStateAction<Settings | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
}): () => void {
  let cancelled = false;
  void (async () => {
    try {
      const s = await options.client.getSettings();
      if (cancelled) return;
      options.setDraft(s);
      options.refs.acknowledgedRef.current = s;
      options.setAcknowledged(s);
      options.refs.onSavedRef.current(s);
    } catch (e) {
      if (!cancelled) options.setError(formatContractError(e));
    } finally {
      if (!cancelled) options.setLoading(false);
    }
  })();
  return () => { cancelled = true; };
}

function useSettingsSync(args: ControllerArgs, ui: UiState, refs: ReturnType<typeof useControllerRefs>, queueVersion: number) {
  const { settings, client } = args;
  const { draft, setDraft, setAcknowledged, setLoading, setError } = ui;
  useEffect(() => mirrorDraftIntoRef({ refs, draft }), [refs, draft]);
  useEffect(
    () => syncParentFeedback({ settings, refs, queueVersion, setDraft, setLoading }),
    [setDraft, setLoading, settings, queueVersion, refs],
  );
  useEffect(() => {
    if (settings) return;
    return loadSettingsWhenMissing({ client, refs, setDraft, setAcknowledged, setError, setLoading });
  }, [client, refs, setDraft, setAcknowledged, setError, setLoading, settings]);
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

// One ordered save queue shared by every controller instance. Each save waits
// for the previous one to settle, so requests and responses are processed in
// global submission order rather than reordered by IPC. Because the queue
// outlives any single controller, a save queued just before an unmount still
// runs in order — a newer save from a remounted controller is therefore always
// the last write (and its `onSaved` callback fires last).
let sharedSaveChain: Promise<void> = Promise.resolve();

// Number of saves submitted but not yet settled, shared across remounts just
// like `sharedSaveChain`. While any save is pending, feedback delivered through
// the parent's `settings` prop reflects an *earlier* save (e.g. save 11's
// `onSaved` arriving while save 13 is still queued). Letting that stale value
// replace the local draft would make `mergePreservingEdits` mistake it for a
// post-submit user edit and leave the UI showing an older value than the one
// persisted. Parent feedback is still honoured once nothing is pending, so
// single-save flows (including the initial load) keep working.
let pendingSaves = 0;

// Queue-drain state published to every controller when the shared save queue
// becomes empty (pendingSaves reaches 0 on BOTH success and failure). The sync
// effect subscribes to this version so it reruns after the last save settles —
// including when that save rejected and produced no `onSaved` — and can replay
// the parent feedback it had to skip while saves were in flight. A plain
// `settings` prop change is not enough to trigger that replay: if the last save
// fails, the parent value never changes again and the effect would otherwise
// never rerun, leaving a remounted controller showing a stale value.
let saveQueueVersion = 0;
const queueDrainListeners = new Set<() => void>();

// The app shell's Elevate control has no access to the mounted Settings
// controller's draft. The controller registers a flusher while mounted so the
// shell can persist that draft through the same save queue before restarting
// (#92). A stale registration is cleared on unmount.
//
// The shell mounts exactly one SettingsView at a time: SettingsPanel renders it
// only while the settings view is active and returns null otherwise, which
// unmounts the previous controller. So at most one controller ever owns this
// slot and a newer mount cannot overwrite a live registration. No app-shell
// state consolidation is needed.
let draftFlusher: (() => Promise<boolean>) | null = null;

// The same mounted controller publishes whether its draft differs from what
// the backend acknowledged. The window-close handshake reads this at request
// time to decide between an instant clean exit and a save/cancel prompt (#93).
// This is deliberately a reader rather than a reuse of the #91 `settingsDirty`
// navigation flag: that flag is render-time state published on every edit and
// would be stale inside the long-lived close subscription, while this reader
// consults draftRef/acknowledgedRef (the same source of truth) at call time.
// Both flags therefore describe one state and cannot diverge.
let draftDirtyReader: (() => boolean) | null = null;

export function registerDraftFlusher(flusher: (() => Promise<boolean>) | null): void {
  draftFlusher = flusher;
}

export function registerDraftDirtyReader(reader: (() => boolean) | null): void {
  draftDirtyReader = reader;
}

// Returns whether it is safe to relaunch: true when the draft was persisted
// (or there was nothing to persist), false when persistence failed and the
// caller must keep the draft and abort the restart (#92).
export async function flushPendingDraft(): Promise<boolean> {
  const flusher = draftFlusher;
  return flusher ? flusher() : true;
}

// True only while a mounted Settings controller holds a draft that the backend
// has not acknowledged. Reads the controller's refs at call time so the close
// handshake sees the latest edit rather than a stale render (#93).
export function isPendingDraftDirty(): boolean {
  return draftDirtyReader ? draftDirtyReader() : false;
}

function subscribeQueueDrain(listener: () => void): () => void {
  queueDrainListeners.add(listener);
  return () => { queueDrainListeners.delete(listener); };
}

function getSaveQueueVersion(): number {
  return saveQueueVersion;
}

// Await the shared save queue draining so a dirty check or flush observes the
// value the backend actually acknowledged, not a snapshot from before an
// in-flight save settled. An edit back to the acknowledged value while a
// different value is still being persisted is not a clean draft (#93).
function waitForPendingSaves(): Promise<void> {
  if (pendingSaves === 0) return Promise.resolve();
  return new Promise((resolve) => {
    let off: (() => void) | null = null;
    const check = () => {
      if (pendingSaves !== 0) return;
      off?.();
      resolve();
    };
    off = subscribeQueueDrain(check);
    check();
  });
}

function settleSave(): void {
  pendingSaves -= 1;
  if (pendingSaves === 0) {
    saveQueueVersion += 1;
    for (const listener of queueDrainListeners) listener();
  }
}

function useSaveHandler(client: SweeperClient, ui: UiState, refs: ReturnType<typeof useControllerRefs>) {
  const { setDraft, setAcknowledged, setError, setInfo, setSaving } = ui;
  // Single-flight guard: at most one `updateSettings` IPC call is in flight.
  // `toSave` is captured synchronously when the save is requested, not when the
  // queued task runs. Edits that land after a save was submitted stay in the
  // draft and are preserved by `mergePreservingEdits` when that save's result
  // is merged, so the backend and UI converge on one consistent final state.
  // Resolves to whether the backend actually persisted the value: the Elevate
  // path uses that to abort before restarting on a failure (#92).
  return useCallback((next?: Settings): Promise<boolean> => {
    const toSave = next ?? refs.draftRef.current;
    if (!toSave) return Promise.resolve(true);
    refs.draftRef.current = toSave;
    // Remember exactly what was submitted so reconciliation after a drain can
    // tell a genuine post-submit edit from deferred parent feedback.
    refs.lastSubmittedRef.current = toSave;
    setDraft(toSave);
    pendingSaves += 1;
    const run = sharedSaveChain.then(async (): Promise<boolean> => {
      setSaving(true);
      setError(null);
      setInfo(null);
      let persisted = false;
      try {
        const saved = await client.updateSettings(toSave);
        // The backend acknowledged exactly `saved`. That may differ from the
        // merged draft when a newer edit landed while this save was in flight,
        // so tracking it separately keeps a later flush honest (#92).
        refs.acknowledgedRef.current = saved;
        setAcknowledged(saved);
        const after = refs.draftRef.current ?? saved;
        const merged = mergePreservingEdits({ saved, submitted: toSave, after });
        refs.draftRef.current = merged;
        setDraft(merged);
        refs.onSavedRef.current(merged);
        setInfo('Settings saved. Active sessions may be invalidated when scan-affecting settings change.');
        persisted = true;
      } catch (e) {
        setError(formatContractError(e));
      } finally {
        // Decrement only after onSaved has delivered this save's feedback, so
        // any parent feedback still observed while a later save is queued is
        // correctly treated as stale by the sync effect. When this was the last
        // outstanding save, settleSave also publishes a new queue-drain version
        // (on failure as well as success) so the sync effect replays deferred
        // feedback even though no onSaved followed the rejected save.
        settleSave();
        setSaving(false);
      }
      return persisted;
    });
    // Keep the shared chain alive after a rejected task so later saves still run.
    sharedSaveChain = run.then(() => {}, () => {});
    return run;
  }, [client, refs, setAcknowledged, setDraft, setError, setInfo, setSaving]);
}

function useCustomPathActions(input: { client: SweeperClient; ui: UiState; refs: ReturnType<typeof useControllerRefs>; save: (next?: Settings) => Promise<boolean> }) {
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

function useRelaunchAction(client: SweeperClient, flushDraft: () => Promise<boolean>) {
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchInfo, setRelaunchInfo] = useState<string | null>(null);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const runAsAdministrator = useCallback(async () => {
    setRelaunching(true);
    setRelaunchInfo(null);
    setRelaunchError(null);
    try {
      // Persist the pending draft through the shared save queue before the
      // restart is requested, so no edit is lost (#92). A failed save must
      // abort the restart and keep the draft intact.
      const persisted = await flushDraft();
      if (!persisted) {
        setRelaunchError('Settings could not be saved. Elevate was cancelled; your changes are kept.');
        return;
      }
      const res = await client.relaunchElevated();
      if (res.launched) setRelaunchInfo('Administrator launch requested. Approve the Windows elevation prompt to continue in the elevated session.');
      else setRelaunchError('Administrator launch was not started.');
    } catch (e) {
      setRelaunchError(formatContractError(e));
    } finally {
      setRelaunching(false);
    }
  }, [client, flushDraft]);
  return { relaunching, relaunchInfo, relaunchError, runAsAdministrator };
}

export function useSettingsController(args: ControllerArgs) {
  const ui = useUiState(args.settings);
  const [highlightAllowUnknown, setHighlightAllowUnknown] = useState(false);
  const allowUnknownRowRef = useRef<HTMLDivElement | null>(null);
  const refs = useControllerRefs(args);
  const queueVersion = useSyncExternalStore(subscribeQueueDrain, getSaveQueueVersion, getSaveQueueVersion);
  useSettingsSync(args, ui, refs, queueVersion);
  const save = useSaveHandler(args.client, ui, refs);
  const paths = useCustomPathActions({ client: args.client, ui, refs, save });
  useFocusAndPrefill({
    args,
    refs,
    setCustomPathInput: paths.setCustomPathInput,
    allowUnknownRowRef,
    setHighlight: setHighlightAllowUnknown,
  });
  // Compare the editable draft with what the backend actually acknowledged,
  // not with parent feedback (which may carry a post-submit edit merged by the
  // save handler). This keeps a newer edit dirty so Elevate flushes it (#92).
  const isDirty = useMemo(() => !settingsEqual(ui.draft, ui.acknowledged), [ui.draft, ui.acknowledged]);
  // Only a genuinely dirty draft is saved: Elevate with no edits must not bump
  // the settings generation or invalidate live sessions (#92). Settle any
  // outstanding save first, then compare the current draft against the value
  // that actually landed and flush a corrective change if it differs (#93).
  const flushDraft = useCallback(async (): Promise<boolean> => {
    await waitForPendingSaves();
    if (settingsEqual(refs.draftRef.current, refs.acknowledgedRef.current)) return true;
    const persisted = await save();
    await waitForPendingSaves();
    return persisted;
  }, [refs, save]);
  useEffect(() => {
    registerDraftFlusher(flushDraft);
    // An outstanding save is itself a reason to treat the draft as dirty: its
    // acknowledgement has not landed yet, so a rendered equality of draft and
    // acknowledged cannot prove the backend agrees (#93).
    registerDraftDirtyReader(
      () => pendingSaves > 0 || !settingsEqual(refs.draftRef.current, refs.acknowledgedRef.current),
    );
    return () => {
      registerDraftFlusher(null);
      registerDraftDirtyReader(null);
    };
  }, [flushDraft, refs]);
  const relaunch = useRelaunchAction(args.client, flushDraft);
  const deepDisabled = !!ui.draft && !ui.draft.enableTier3;
  const { setDraft } = ui;
  const resetDraft = useCallback(() => { if (args.settings) setDraft(args.settings); }, [args.settings, setDraft]);
  return {
    draft: ui.draft, setDraft: ui.setDraft, loading: ui.loading, saving: ui.saving, error: ui.error, info: ui.info,
    customPathInput: paths.customPathInput, setCustomPathInput: paths.setCustomPathInput, validatingPath: paths.validatingPath,
    relaunching: relaunch.relaunching, relaunchInfo: relaunch.relaunchInfo, relaunchError: relaunch.relaunchError,
    highlightAllowUnknown, allowUnknownRowRef, isDirty, deepDisabled,
    save, flushDraft, addCustomPath: paths.addCustomPath, removeCustomPath: paths.removeCustomPath, runAsAdministrator: relaunch.runAsAdministrator, applyCustomPathPreset: paths.applyCustomPathPreset, resetDraft,
  };
}