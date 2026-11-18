import { useMemo } from 'react';
import type { TargetScanResultDto } from '../../contract/bindings';
import { buildSelectionIndex, selectedBytesFromIndex, selectedCategoryCountFromIndex, selectionNeedsConfirmationFromIndex } from './previewPerf';
import { countSelectable, countSkipped, eligibleSafeIdsFor, heroReclaimableBytes, visibleEligibleSafeIdsFor } from './scanViewModel';
import type { useScanWorkflow } from './useScanWorkflow';

type Workflow = ReturnType<typeof useScanWorkflow>;

function hasVersion(args: { value: number | null | undefined }): boolean {
  return args.value != null;
}

function settingsStale(args: { workflow: Workflow; settingsVersion?: number }): boolean {
  if (!args.workflow.results) return false;
  if (!hasVersion({ value: args.workflow.scanSettingsVersion })) return false;
  if (!hasVersion({ value: args.settingsVersion })) return false;
  return args.workflow.scanSettingsVersion !== args.settingsVersion;
}

function isFilterActive(toolbar: Workflow['toolbar']): boolean {
  if (toolbar.filter !== 'all') return true;
  return toolbar.query.trim().length > 0;
}

function selectSafeIds(args: { active: boolean; visible: string[]; all: string[] }): string[] {
  if (args.active) return args.visible;
  return args.all;
}

function allSelected(ids: string[], selected: Set<string>): boolean {
  if (ids.length === 0) return false;
  return ids.every((id) => selected.has(id));
}

export function useScanDerived(w: Workflow, settingsVersion?: number) {
  const allow = w.allowDeleteUnknown;
  const targets = w.visibleBaseTargets;
  const sel = w.selectedIds;
  const selectableCount = useMemo(() => countSelectable({ targets, allow }), [targets, allow]);
  const skippedCount = useMemo(() => countSkipped({ targets, allow }), [targets, allow]);
  const selectionIndex = useMemo(() => buildSelectionIndex({ targets }), [targets]);
  const selectedBytes = useMemo(() => selectedBytesFromIndex({ index: selectionIndex, selected: sel }), [selectionIndex, sel]);
  const requiresConfirmation = useMemo(() => selectionNeedsConfirmationFromIndex({ index: selectionIndex, selected: sel }), [selectionIndex, sel]);
  const eligibleSafeIds = useMemo(() => eligibleSafeIdsFor({ targets, allow }), [targets, allow]);
  const allEligibleSelected = allSelected(eligibleSafeIds, sel);
  const selectedCategoryCountValue = useMemo(() => selectedCategoryCountFromIndex({ index: selectionIndex, selected: sel }), [selectionIndex, sel]);
  const heroReclaimable = useMemo(() => heroReclaimableBytes({ targets }), [targets]);
  const stale = settingsStale({ workflow: w, settingsVersion });
  const active = isFilterActive(w.toolbar);
  const visibleEligibleSafeIds = useMemo(() => visibleEligibleSafeIdsFor({ visibleTargets: w.visibleTargets, allow }), [w.visibleTargets, allow]);
  const selectIds = selectSafeIds({ active, visible: visibleEligibleSafeIds, all: eligibleSafeIds });
  const allSelectSafeSelected = allSelected(selectIds, sel);
  return { allow, targets, sel, selectableCount, skippedCount, selectedBytes, requiresConfirmation, eligibleSafeIds, allEligibleSelected, selectedCategoryCount: selectedCategoryCountValue, heroReclaimable, settingsStale: stale, isFilterActive: active, selectSafeIds: selectIds, allSelectSafeSelected };
}