import { useMemo } from 'react';
import type { TargetScanResultDto } from '../../contract/bindings';
import { sumDecimalStrings } from '../../utils/format';
import { countSelectable, countSkipped, eligibleSafeIdsFor, heroReclaimableBytes, needsConfirmation, selectedCategoryCount, visibleEligibleSafeIdsFor } from './scanViewModel';
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
  const selectedBytes = useMemo(() => sumDecimalStrings({ values: targets.flatMap((t) => t.items.filter((i) => sel.has(i.id)).map((i) => i.size)) }), [targets, sel]);
  const requiresConfirmation = useMemo(() => needsConfirmation({ targets, allow, selectedIds: sel }), [targets, allow, sel]);
  const eligibleSafeIds = useMemo(() => eligibleSafeIdsFor({ targets, allow }), [targets, allow]);
  const allEligibleSelected = allSelected(eligibleSafeIds, sel);
  const selectedCategoryCountValue = useMemo(() => selectedCategoryCount({ targets, allow, selectedIds: sel }), [targets, allow, sel]);
  const heroReclaimable = useMemo(() => heroReclaimableBytes({ targets }), [targets]);
  const stale = settingsStale({ workflow: w, settingsVersion });
  const active = isFilterActive(w.toolbar);
  const visibleEligibleSafeIds = useMemo(() => visibleEligibleSafeIdsFor({ visibleTargets: w.visibleTargets, allow }), [w.visibleTargets, allow]);
  const selectIds = selectSafeIds({ active, visible: visibleEligibleSafeIds, all: eligibleSafeIds });
  const allSelectSafeSelected = allSelected(selectIds, sel);
  return { allow, targets, sel, selectableCount, skippedCount, selectedBytes, requiresConfirmation, eligibleSafeIds, allEligibleSelected, selectedCategoryCount: selectedCategoryCountValue, heroReclaimable, settingsStale: stale, isFilterActive: active, selectSafeIds: selectIds, allSelectSafeSelected };
}