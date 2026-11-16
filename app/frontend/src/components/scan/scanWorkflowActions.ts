import type { ScanResults, SweeperClient } from '../../api/client';
import type { TargetScanResultDto } from '../../contract/bindings';
import { isSelectableScanItem, visibleBaseTargets as getVisibleBaseTargets } from './scanHelpers';

export function autoSelectSafe(data: ScanResults, allowDeleteUnknown: boolean): Set<string> {
  const auto = new Set<string>();
  for (const t of data.targets) {
    if (t.risk !== 'Safe') continue;
    for (const i of t.items) {
      if (isSelectableScanItem({ item: i, target: t, allowDeleteUnknown })) auto.add(i.id);
    }
  }
  return auto;
}

export function defaultCollapsed(baseVisible: TargetScanResultDto[]): Set<string> {
  const collapsed = new Set<string>();
  const primary = baseVisible.find((t) => t.targetName === 'User Temp') ?? baseVisible[0] ?? null;
  for (const t of baseVisible) {
    if (primary && t.targetId === primary.targetId) continue;
    collapsed.add(t.targetId);
  }
  return collapsed;
}

export function primaryScanItem(data: ScanResults): string | null {
  const baseVisible = getVisibleBaseTargets(data.targets);
  const primary = baseVisible.find((t) => t.targetName === 'User Temp') ?? baseVisible[0] ?? null;
  const first = primary?.items[0] ?? data.targets.flatMap((t) => t.items)[0];
  return first?.id ?? null;
}

export function scanTiers(tier1: boolean, tier2: boolean): ('Tier1' | 'Tier2')[] | null {
  const tiers = [tier1 ? 'Tier1' : null, tier2 ? 'Tier2' : null].filter(Boolean) as ('Tier1' | 'Tier2')[];
  return tiers.length ? tiers : null;
}

export function progressFromEvent(ev: { kind: string; current_target_name?: string | null; completed_targets?: number; total_targets?: number; scanned_items?: string; scanned_bytes?: string }) {
  if (ev.kind === 'started') return { status: 'Scan in progress…', progress: { completedTargets: 0, totalTargets: 0, currentTargetName: null, scannedItems: '0', scannedBytes: '0' } };
  if (ev.kind === 'progress') return { status: ev.current_target_name ? `Scanning ${ev.current_target_name}…` : 'Scan in progress…', progress: { completedTargets: ev.completed_targets ?? 0, totalTargets: ev.total_targets ?? 0, currentTargetName: ev.current_target_name ?? null, scannedItems: ev.scanned_items ?? '0', scannedBytes: ev.scanned_bytes ?? '0' } };
  if (ev.kind === 'cancelled') return { status: 'Scan cancelled.', progress: null };
  if (ev.kind === 'completed') return { status: '', progress: null };
  if (ev.kind === 'failed') return { status: 'Scan failed.', progress: null };
  return null;
}

export async function inspectById(args: { client: SweeperClient; itemId: string; reqId: number; ref: { current: number }; setInspected: (v: unknown) => void }) {
  try {
    const r = await args.client.explainItem(args.itemId);
    if (args.ref.current === args.reqId) args.setInspected(r);
  } catch { /* ignore */ }
}
