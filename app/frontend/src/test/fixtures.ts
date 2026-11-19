import type { AnalyzerNodeDto, ScanItemDto, TargetScanResultDto } from '../contract/bindings';
import type { ScanResults, Settings, StartAnalyzerResult } from '../api/client';

export function makeSettings(options: Partial<Settings> = {}): Settings {
  return {
    enableTier1: true,
    enableTier2: true,
    enableTier3: false,
    enableElevationTargets: false,
    minAgeHours: 24,
    allowDeleteUnknown: false,
    customPaths: [],
    staleProjectDays: 30,
    ...options,
  };
}

export function makeScanItem(options: { id: string; path: string; size?: string; targetId?: string; ageEligible?: boolean; mtimeUnixSecs?: string }): ScanItemDto {
  return {
    id: options.id,
    targetId: options.targetId ?? 't-safe',
    path: options.path,
    size: options.size ?? '100',
    mtimeUnixSecs: options.mtimeUnixSecs ?? '1700000000',
    ageEligible: options.ageEligible ?? true,
  };
}

export function makeScanTarget(options: {
  targetId: string;
  targetName: string;
  risk: TargetScanResultDto['risk'];
  tier?: TargetScanResultDto['tier'];
  items: ScanItemDto[];
  resolvedPaths?: string[];
  statusReason?: string | null;
  scanned?: boolean;
}): TargetScanResultDto {
  const totalBytes = options.items.reduce((a, i) => a + BigInt(i.size || '0'), 0n).toString();
  const eligibleItems = options.items.filter((i) => i.ageEligible);
  return {
    targetId: options.targetId,
    targetName: options.targetName,
    tier: options.tier ?? 'Tier1',
    risk: options.risk,
    resolvedPaths: options.resolvedPaths ?? ['C:\\Temp'],
    scanned: options.scanned ?? true,
    statusReason: options.statusReason ?? null,
    totalBytes,
    eligibleBytes: eligibleItems.reduce((a, i) => a + BigInt(i.size || '0'), 0n).toString(),
    fileCount: String(options.items.length),
    eligibleCount: String(eligibleItems.length),
    skippedCount: String(options.items.length - eligibleItems.length),
    items: options.items,
  };
}

export function makeScanResults(options: { sessionId?: string; minAgeHours?: number; targets: TargetScanResultDto[] }): ScanResults {
  return {
    sessionId: options.sessionId ?? 'scan-1',
    startedUnixSecs: '1710000000',
    minAgeHours: options.minAgeHours ?? 24,
    targets: options.targets,
  };
}

export function makeAnalyzerNode(options: {
  id: string;
  name: string;
  path: string;
  isDir?: boolean;
  sizeBytes?: string;
  itemCount?: string;
  risk?: AnalyzerNodeDto['risk'];
  selectable?: boolean;
  displayOnly?: boolean;
  children?: AnalyzerNodeDto[];
  tier?: AnalyzerNodeDto['tier'];
  targetId?: string | null;
  isLinkPoint?: boolean;
  isCloudPlaceholder?: boolean;
  description?: string;
  reason?: string | null;
}): AnalyzerNodeDto {
  return {
    id: options.id,
    name: options.name,
    path: options.path,
    isDir: options.isDir ?? false,
    sizeBytes: options.sizeBytes ?? '0',
    itemCount: options.itemCount ?? '0',
    risk: options.risk ?? 'Safe',
    description: options.description ?? 'desc',
    reason: options.reason ?? null,
    tier: options.tier ?? null,
    targetId: options.targetId ?? null,
    isLinkPoint: options.isLinkPoint ?? false,
    isCloudPlaceholder: options.isCloudPlaceholder ?? false,
    displayOnly: options.displayOnly ?? false,
    selectable: options.selectable ?? false,
    children: options.children ?? [],
  };
}

export function makeAnalyzerResult(options: { sessionId?: string; rootPath?: string; root: AnalyzerNodeDto; totalBytes?: string; totalItems?: string }): StartAnalyzerResult {
  return {
    sessionId: options.sessionId ?? 'an-1',
    rootPath: options.rootPath ?? options.root.path,
    totalBytes: options.totalBytes ?? options.root.sizeBytes,
    totalItems: options.totalItems ?? options.root.itemCount,
    scannedDirs: '1',
    scannedFiles: options.root.itemCount,
    cancelled: false,
    warnings: [],
    root: options.root,
  };
}
