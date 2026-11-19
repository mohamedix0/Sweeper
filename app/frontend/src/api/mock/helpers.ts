import type { AnalyzerNodeDto, PreviewTargetBreakdownDto, RiskDto, ScanItemDto, SkippedReasonDto, TargetScanResultDto, TierDto } from '../../contract/bindings';
import type { ContractError, Settings } from '../client';
import type { ScanItemInfo } from './data';

export function err(input: { code: 'unknownItem'; count: string } | { code: Exclude<ContractError['code'], 'unknownItem'>; reason: string }): never {
  if (input.code === 'unknownItem') throw { code: input.code, details: { count: input.count } } satisfies ContractError;
  throw { code: input.code, details: { reason: input.reason } } satisfies ContractError;
}

export function enabledTiers(settingsState: Settings, tiers: TierDto[] | null): Set<TierDto> {
  const list = tiers ?? ([settingsState.enableTier1 ? 'Tier1' : null, settingsState.enableTier2 ? 'Tier2' : null, settingsState.enableTier3 ? 'Tier3' : null].filter(Boolean) as TierDto[]);
  return new Set(list);
}

export function normalizeMockPath(path: string): string {
  return path.replace(/[\\/]+$/, '').toLowerCase();
}

export function isAncestorMockPath(input: { ancestorPath: string; descendantPath: string }): boolean {
  const a = normalizeMockPath(input.ancestorPath);
  const d = normalizeMockPath(input.descendantPath);
  if (!a) return false;
  if (!d) return false;
  if (a === d) return false;
  const prefix = a.endsWith('\\') ? a : `${a}\\`;
  return d.startsWith(prefix);
}

export function dedupeAnalyzerNodes(nodes: AnalyzerNodeDto[]): AnalyzerNodeDto[] {
  return nodes.filter((node, _, all) => {
    const ancestor = all.some((other) => {
      if (other.id === node.id) return false;
      return isAncestorMockPath({ ancestorPath: other.path, descendantPath: node.path });
    });
    return !ancestor;
  });
}

export function analyzerBreakdown(selected: AnalyzerNodeDto[]): PreviewTargetBreakdownDto[] {
  const deduped = dedupeAnalyzerNodes(selected);
  const by = new Map<string, { targetId: string; targetName: string; count: bigint; bytes: bigint }>();
  for (const n of deduped) {
    const targetId = n.targetId ?? 'custom-path';
    const targetName = targetId === 'custom-path' ? 'Custom Path' : (n.targetId ?? n.name);
    const e = by.get(targetId) ?? { targetId, targetName, count: 0n, bytes: 0n };
    e.count += 1n; e.bytes += BigInt(n.sizeBytes); by.set(targetId, e);
  }
  return [...by.values()].map((e) => ({ targetId: e.targetId, targetName: e.targetName, selectedCount: e.count.toString(), selectedBytes: e.bytes.toString() }));
}

export function scanBreakdown(selected: Array<{ item: { targetId: string; targetName: string; size: string } }>) {
  const byTarget = new Map<string, { targetId: string; targetName: string; count: bigint; bytes: bigint }>();
  let totalBytes = 0n;
  for (const { item } of selected) {
    const entry = byTarget.get(item.targetId) ?? { targetId: item.targetId, targetName: item.targetName, count: 0n, bytes: 0n };
    entry.count += 1n; entry.bytes += BigInt(item.size); byTarget.set(item.targetId, entry); totalBytes += BigInt(item.size);
  }
  const breakdown: PreviewTargetBreakdownDto[] = [...byTarget.values()].map((e) => ({ targetId: e.targetId, targetName: e.targetName, selectedCount: e.count.toString(), selectedBytes: e.bytes.toString() }));
  return { breakdown, totalBytes };
}

export function toSkippedReasons(reasons: Record<string, bigint>): SkippedReasonDto[] {
  return Object.entries(reasons).map(([reason, count]) => ({ reason, count: count.toString() }));
}

export function sumNodeBytes(nodes: AnalyzerNodeDto[]): string {
  return nodes.reduce((a, n) => a + BigInt(n.sizeBytes), 0n).toString();
}

export function isUnknownBlocked(input: { risk: ScanItemInfo['risk'] | undefined; allowDeleteUnknown: boolean }): boolean {
  if (input.risk !== 'Unknown') return false;
  return !input.allowDeleteUnknown;
}
