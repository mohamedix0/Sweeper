import type { ScanResults, SweeperClient } from '../../api/client';
import type { TargetScanResultDto } from '../../contract/bindings';
import { failedExplanation, type ExplanationFallback } from '../cleanShared';
import { isSelectableScanItem, visibleBaseTargets } from './scanHelpers';

function maybeAddSafeItem(args: { item: TargetScanResultDto['items'][number]; target: TargetScanResultDto; allowDeleteUnknown: boolean; auto: Set<string> }) {
  if (isSelectableScanItem({ item: args.item, target: args.target, allowDeleteUnknown: args.allowDeleteUnknown })) args.auto.add(args.item.id);
}

function addSafeItems(args: { target: TargetScanResultDto; allowDeleteUnknown: boolean; auto: Set<string> }) {
  if (args.target.risk !== 'Safe') return;
  for (const item of args.target.items) maybeAddSafeItem({ item, target: args.target, allowDeleteUnknown: args.allowDeleteUnknown, auto: args.auto });
}

export function autoSelectSafe(args: { data: ScanResults; allowDeleteUnknown: boolean }): Set<string> {
  const auto = new Set<string>();
  for (const target of args.data.targets) addSafeItems({ target, allowDeleteUnknown: args.allowDeleteUnknown, auto });
  return auto;
}

function primaryTarget(baseVisible: TargetScanResultDto[]): TargetScanResultDto | null {
  return baseVisible.find((t) => t.targetName === 'User Temp') ?? baseVisible[0] ?? null;
}

function shouldCollapseTarget(args: { target: TargetScanResultDto; primary: TargetScanResultDto | null }): boolean {
  if (!args.primary) return true;
  return args.target.targetId !== args.primary.targetId;
}

export function defaultCollapsed(args: { baseVisible: TargetScanResultDto[] }): Set<string> {
  const collapsed = new Set<string>();
  const primary = primaryTarget(args.baseVisible);
  const toCollapse = args.baseVisible.filter((target) => shouldCollapseTarget({ target, primary }));
  for (const target of toCollapse) collapsed.add(target.targetId);
  return collapsed;
}

function firstItemInTargets(targets: TargetScanResultDto[]): string | null {
  const found = targets.map((target) => target.items[0]?.id ?? null).find((id): id is string => id != null);
  return found ?? null;
}

export function primaryScanItem(args: { data: ScanResults }): string | null {
  const baseVisible = visibleBaseTargets({ targets: args.data.targets });
  const primary = primaryTarget(baseVisible);
  const primaryFirst = primary?.items[0]?.id ?? null;
  if (primaryFirst) return primaryFirst;
  return firstItemInTargets(args.data.targets);
}

type ScanTier = 'Tier1' | 'Tier2' | 'Tier3';

export function scanTiers(args: { tier1: boolean; tier2: boolean; enableTier3: boolean }): ScanTier[] {
  const tiers: ScanTier[] = [];
  if (args.tier1) tiers.push('Tier1');
  if (args.tier2) tiers.push('Tier2');
  if (args.enableTier3) tiers.push('Tier3');
  return tiers;
}

type ProgressEvent = { kind: string; current_target_name?: string | null; completed_targets?: number; total_targets?: number; scanned_items?: string; scanned_bytes?: string };

function startedProgress() {
  return { status: 'Scan in progress…', progress: { completedTargets: 0, totalTargets: 0, currentTargetName: null, scannedItems: '0', scannedBytes: '0' } };
}

function progressStatus(name?: string | null): string {
  if (name) return `Scanning ${name}…`;
  return 'Scan in progress…';
}

function progressValue(ev: ProgressEvent) {
  return {
    completedTargets: ev.completed_targets ?? 0,
    totalTargets: ev.total_targets ?? 0,
    currentTargetName: ev.current_target_name ?? null,
    scannedItems: ev.scanned_items ?? '0',
    scannedBytes: ev.scanned_bytes ?? '0',
  };
}

function progressUpdate(ev: ProgressEvent) {
  return { status: progressStatus(ev.current_target_name), progress: progressValue(ev) };
}

export function progressFromEvent(ev: ProgressEvent) {
  if (ev.kind === 'started') return startedProgress();
  if (ev.kind === 'progress') return progressUpdate(ev);
  if (ev.kind === 'cancelled') return { status: 'Scan cancelled.', progress: null };
  if (ev.kind === 'completed') return { status: '', progress: null };
  if (ev.kind === 'failed') return { status: 'Scan failed.', progress: null };
  return null;
}

function applyInspectResult(args: { reqId: number; ref: { current: number }; setInspected: (v: unknown) => void; result: unknown }) {
  if (args.ref.current === args.reqId) args.setInspected(args.result);
}

export async function inspectById(args: { client: SweeperClient; itemId: string; reqId: number; ref: { current: number }; setInspected: (v: unknown) => void; known?: ExplanationFallback }) {
  try {
    const result = await args.client.explainItem(args.itemId);
    applyInspectResult({ reqId: args.reqId, ref: args.ref, setInspected: args.setInspected, result });
  } catch (error) {
    // Never leave the inspector empty on a failed explain: keep the known
    // identity/Risk visible and let the panel surface the explicit notice.
    console.warn(`Sweeper: explanation unavailable for scan item ${args.itemId}.`, error);
    applyInspectResult({ reqId: args.reqId, ref: args.ref, setInspected: args.setInspected, result: failedExplanation({ itemId: args.itemId, known: args.known }) });
  }
}
