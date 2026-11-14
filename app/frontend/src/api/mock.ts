import type {
  AnalyzerNodeDto,
  PreviewTargetBreakdownDto,
  RiskDto,
  ScanItemDto,
  SkippedReasonDto,
  TargetScanResultDto,
  TierDto,
} from '../contract/bindings';
import type { Settings, SweeperClient } from './client';

export type CleanProgressEvent = {
  previewToken: string;
  targetId: string;
  completedItems: string;
  totalItems: string;
  bytesFreedSoFar: string;
  status: string;
};

export type ExecuteCleanResponse = {
  bytesFreed: string;
  deletedCount: string;
  skippedCount: string;
  skippedReasons: SkippedReasonDto[];
  aborted: boolean;
  abortReason: string | null;
  logId: string | null;
};

export type ContractError =
  | { code: 'sessionInvalid'; details: { reason: string } }
  | { code: 'unknownItem'; details: { count: string } }
  | { code: 'previewTokenInvalid'; details: { reason: string } }
  | { code: 'invalidInput'; details: { reason: string } }
  | { code: 'notFound'; details: { reason: string } }
  | { code: 'internal'; details: { reason: string } };

type ScanProgress =
  | { kind: 'started'; session_id: string }
  | { kind: 'completed'; session_id: string }
  | { kind: 'failed'; session_id: string; error: ContractError };

export type AnalyzerProgressEvent = {
  sessionId: string;
  scannedDirs: string;
  scannedFiles: string;
  scannedBytes: string;
  currentPath: string | null;
};

export type ScanResults = {
  sessionId: string;
  startedUnixSecs: string;
  minAgeHours: number;
  targets: TargetScanResultDto[];
};

export type StartAnalyzerResult = {
  sessionId: string;
  rootPath: string;
  totalBytes: string;
  totalItems: string;
  scannedDirs: string;
  scannedFiles: string;
  cancelled: boolean;
  warnings: string[];
  root: AnalyzerNodeDto;
};

export type PreviewCleanResult = {
  previewToken: string;
  breakdown: PreviewTargetBreakdownDto[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
};

export type ExplainResult = { itemId: string; title: string; description: string; risk: RiskDto };

export type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
export type LogContent = { id: string; content: string };

type ScanItemInfo = ScanItemDto & { targetId: string; risk: RiskDto; targetName: string; description: string };

type SessionState = {
  id: string;
  createdAtMs: number;
  minAgeHours: number;
  targets: TargetScanResultDto[];
  itemsById: Map<string, ScanItemInfo>;
  settingsGeneration: number;
};

type AnalyzerSessionState = {
  id: string;
  createdAtMs: number;
  rootPath: string;
  result: StartAnalyzerResult;
  nodesById: Map<string, AnalyzerNodeDto>;
  settingsGeneration: number;
};

type PreviewState = {
  token: string;
  sessionId: string;
  createdAtMs: number;
  used: boolean;
  itemIds: string[];
  settingsGeneration: number;
  previewEstimatedBytes: string;
};

type LogEntry = { id: string; name: string; modifiedUnixSecs: string; sizeBytes: string; content: string };

export type MockController = SweeperClient & {
  advanceTimeMs: (ms: number) => void;
  invalidateSettings: () => void;
  setElevationStatus: (status: { isElevated: boolean }) => void;
  setRelaunchResult: (result: { launched: boolean } | { error: ContractError }) => void;
  reset: () => void;
};

function err(code: ContractError['code'], reasonOrCount: string): never {
  if (code === 'unknownItem') {
    throw { code, details: { count: reasonOrCount } } satisfies ContractError;
  }
  throw { code, details: { reason: reasonOrCount } } as ContractError;
}

const SAMPLE_ITEMS_T1: Array<Omit<ScanItemDto, 'id'>> = [
  { targetId: 't-user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp', size: '1048576', mtimeUnixSecs: '1710000000', ageEligible: true },
  { targetId: 't-user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\recent.log', size: '2048', mtimeUnixSecs: '1899999999', ageEligible: false },
  { targetId: 't-crash', path: 'C:\\Users\\Alice\\AppData\\Local\\CrashDumps\\app.dmp', size: '5242880', mtimeUnixSecs: '1700000000', ageEligible: true },
];
const SAMPLE_ITEMS_T2: Array<Omit<ScanItemDto, 'id'>> = [
  { targetId: 't-npm', path: 'C:\\Users\\Alice\\AppData\\Local\\npm-cache\\package-cache-tarballs', size: '7340032', mtimeUnixSecs: '1705000000', ageEligible: true },
];

function buildTarget(
  targetId: string,
  targetName: string,
  tier: TierDto,
  risk: RiskDto,
  description: string,
  rawItems: Array<Omit<ScanItemDto, 'id' | 'targetId'> & { targetId?: string }>,
): { target: TargetScanResultDto; items: ScanItemInfo[] } {
  const items = rawItems.map((it, i) => ({
    ...it,
    id: `${targetId}-item-${i + 1}`,
    risk,
    targetName,
    description,
  })) as ScanItemInfo[];
  let total = 0n;
  let eligible = 0n;
  let eligibleCount = 0n;
  let skipped = 0n;
  for (const it of items) {
    const sz = BigInt(it.size);
    total += sz;
    if (it.ageEligible) {
      eligible += sz;
      eligibleCount += 1n;
    } else {
      skipped += 1n;
    }
  }
  const target: TargetScanResultDto = {
    targetId,
    targetName,
    tier,
    risk,
    resolvedPaths: items.length ? [items[0].path.replace(/\\[^\\]+$/, '')] : [],
    scanned: true,
    statusReason: null,
    totalBytes: total.toString(),
    eligibleBytes: eligible.toString(),
    fileCount: BigInt(items.length).toString(),
    eligibleCount: eligibleCount.toString(),
    skippedCount: skipped.toString(),
    items: items.map(({ risk: _r, targetName: _n, description: _d, ...rest }) => rest),
  };
  return { target, items };
}

function makeNode(partial: Omit<AnalyzerNodeDto, 'children'> & { children?: AnalyzerNodeDto[] }): AnalyzerNodeDto {
  return { ...partial, children: partial.children ?? [] };
}

function buildAnalyzerTree(rootPath: string, allowUnknown: boolean): AnalyzerNodeDto {
  const base = rootPath.replace(/[\\/]+$/, '');
  const deepChildren: AnalyzerNodeDto[] = [];
  const cursorPath = `${base}\\Users\\Alice\\AppData\\Local\\Packages\\DeepVendor`;
  let deepLeaf: AnalyzerNodeDto = makeNode({
    id: 'an-deep-leaf', name: 'cache.bin', path: `${cursorPath}\\layer30\\cache.bin`, isDir: false,
    sizeBytes: '2048', itemCount: '1', risk: 'Safe', description: 'Deep nested cache file (Safe).',
    reason: null, tier: 'Tier2', targetId: 'mock-deep-safe', isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: true, children: [],
  });
  for (let i = 30; i >= 1; i--) {
    deepLeaf = makeNode({
      id: `an-deep-dir-${i}`, name: `layer${i}`, path: `${cursorPath}\\layer${i}`, isDir: true,
      sizeBytes: String(2048 + i), itemCount: String(31 - i), risk: 'Safe',
      description: 'Deep nested directory for virtualization testing.', reason: null, tier: 'Tier2',
      targetId: 'mock-deep-safe', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
      children: [deepLeaf],
    });
  }
  deepChildren.push(deepLeaf);

  const manyFiles: AnalyzerNodeDto[] = [];
  for (let i = 1; i <= 40; i++) {
    manyFiles.push(makeNode({
      id: `an-bulk-file-${i}`,
      name: `temp-${i}.tmp`,
      path: `${base}\\Users\\Alice\\AppData\\Local\\Temp\\bulk\\temp-${i}.tmp`,
      isDir: false, sizeBytes: String(1024 * i), itemCount: '1', risk: 'Safe',
      description: 'Temporary file (Safe).', reason: null, tier: 'Tier1', targetId: 'mock-temp',
      isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
    }));
  }

  const protectedGit = makeNode({
    id: 'an-protected-git', name: '.git', path: `${base}\\Dev\\repo\\.git`, isDir: true,
    sizeBytes: '4096', itemCount: '12', risk: 'Protected', description: 'Protected repository metadata (.git).',
    reason: 'Protected Path', tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: true, selectable: false,
    children: [makeNode({
      id: 'an-protected-git-obj', name: 'objects', path: `${base}\\Dev\\repo\\.git\\objects`, isDir: true,
      sizeBytes: '2048', itemCount: '4', risk: 'Protected', description: 'Protected git internals.',
      reason: 'Protected Path', tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false,
      displayOnly: true, selectable: false,
    })],
  });

  const linkPoint = makeNode({
    id: 'an-link-point', name: 'OneDriveLink', path: `${base}\\Users\\Alice\\OneDriveLink`, isDir: true,
    sizeBytes: '0', itemCount: '0', risk: 'Caution', description: 'Link Point (reparse point). Never followed.',
    reason: 'Link Point — not traversed', tier: null, targetId: null, isLinkPoint: true, isCloudPlaceholder: false,
    displayOnly: true, selectable: false,
  });

  const vhdxNode = makeNode({
    id: 'an-vhdx', name: 'dev-machine.vhdx', path: `${base}\\Users\\Alice\\Virtual Machines\\dev-machine.vhdx`,
    isDir: false, sizeBytes: '1073741824', itemCount: '1', risk: 'Caution',
    description: 'Virtual disk image (.vhdx). Display-only by policy.',
    reason: '.vhdx display-only', tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: true, selectable: false,
  });

  const cloudPlaceholder = makeNode({
    id: 'an-cloud', name: 'CloudPlaceholder.txt', path: `${base}\\Users\\Alice\\OneDrive\\CloudPlaceholder.txt`,
    isDir: false, sizeBytes: '0', itemCount: '1', risk: 'Caution',
    description: 'Cloud placeholder file. Skipped by policy.', reason: 'Cloud placeholder — skipped',
    tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: true, displayOnly: true, selectable: false,
  });

  const unknownSelectable = allowUnknown;
  const unknownReason = allowUnknown ? 'Unknown (selectable when explicitly allowed)' : 'Unknown items are not selectable until allowed in Settings';
  const unknownFolder = makeNode({
    id: 'an-unknown-folder', name: 'MysteryCache', path: `${base}\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache`,
    isDir: true, sizeBytes: '8192', itemCount: '2', risk: 'Unknown',
    description: 'Unknown folder not in Knowledge Base. May require confirmation/policy to remove.',
    reason: unknownReason, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: unknownSelectable,
    children: [makeNode({
      id: 'an-unknown-file', name: 'old.bin', path: `${base}\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache\\old.bin`,
      isDir: false, sizeBytes: '8192', itemCount: '1', risk: 'Unknown',
      description: 'Unknown cached data file.', reason: unknownReason, tier: null, targetId: null,
      isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: unknownSelectable,
    })],
  });

  const cautionFolder = makeNode({
    id: 'an-caution-logs', name: 'Logs', path: `${base}\\ProgramData\\Vendor\\Logs`, isDir: true,
    sizeBytes: '262144', itemCount: '8', risk: 'Caution', description: 'Vendor logs (Caution). Review before removal.',
    reason: null, tier: 'Tier2', targetId: 'mock-caution-logs', isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: true,
    children: [
      makeNode({ id: 'an-caution-log1', name: 'app.log', path: `${base}\\ProgramData\\Vendor\\Logs\\app.log`, isDir: false, sizeBytes: '131072', itemCount: '1', risk: 'Caution', description: 'Application log file.', reason: null, tier: 'Tier2', targetId: 'mock-caution-logs', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true }),
      makeNode({ id: 'an-caution-log2', name: 'service.log', path: `${base}\\ProgramData\\Vendor\\Logs\\service.log`, isDir: false, sizeBytes: '131072', itemCount: '1', risk: 'Caution', description: 'Service log file.', reason: null, tier: 'Tier2', targetId: 'mock-caution-logs', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true }),
    ],
  });

  const safeTempBulk = makeNode({
    id: 'an-safe-temp-bulk', name: 'bulk', path: `${base}\\Users\\Alice\\AppData\\Local\\Temp\\bulk`, isDir: true,
    sizeBytes: manyFiles.reduce((a, n) => a + BigInt(n.sizeBytes), 0n).toString(),
    itemCount: String(manyFiles.length), risk: 'Safe', description: 'Bulk temporary files (Safe).',
    reason: null, tier: 'Tier1', targetId: 'mock-temp', isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: true, children: manyFiles,
  });

  const safeTemp = makeNode({
    id: 'an-safe-temp', name: 'Temp', path: `${base}\\Users\\Alice\\AppData\\Local\\Temp`, isDir: true,
    sizeBytes: (1048576n + 2048n + manyFiles.reduce((a, n) => a + BigInt(n.sizeBytes), 0n)).toString(),
    itemCount: String(2 + manyFiles.length), risk: 'Safe', description: 'User temporary files (Safe).',
    reason: null, tier: 'Tier1', targetId: 'mock-temp', isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: true,
    children: [
      makeNode({ id: 'an-safe-temp-a', name: 'a.tmp', path: `${base}\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp`, isDir: false, sizeBytes: '1048576', itemCount: '1', risk: 'Safe', description: 'Temporary file.', reason: null, tier: 'Tier1', targetId: 'mock-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true }),
      makeNode({ id: 'an-safe-temp-recent', name: 'recent.log', path: `${base}\\Users\\Alice\\AppData\\Local\\Temp\\recent.log`, isDir: false, sizeBytes: '2048', itemCount: '1', risk: 'Safe', description: 'Recent temp log (not age-eligible in scan, but analyzer node shown for selection policy demo).', reason: 'Recent (demo)', tier: 'Tier1', targetId: 'mock-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true }),
      safeTempBulk,
    ],
  });

  const deepBranch = makeNode({
    id: 'an-deep-root', name: 'DeepVendor', path: `${base}\\Users\\Alice\\AppData\\Local\\Packages\\DeepVendor`,
    isDir: true, sizeBytes: '4096', itemCount: '31', risk: 'Safe', description: 'Deep nested structure (virtualization demo).',
    reason: null, tier: 'Tier2', targetId: 'mock-deep-safe', isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: true, children: deepChildren,
  });

  const children = [safeTemp, cautionFolder, unknownFolder, protectedGit, linkPoint, vhdxNode, cloudPlaceholder, deepBranch];
  const totalBytes = children.reduce((a, n) => a + BigInt(n.sizeBytes), 0n).toString();
  const totalItems = children.reduce((a, n) => a + BigInt(n.itemCount), 0n).toString();
  return makeNode({
    id: 'an-root', name: base || rootPath, path: rootPath, isDir: true,
    sizeBytes: totalBytes, itemCount: totalItems, risk: 'Caution',
    description: `Analyzer root for ${rootPath}. Expand to review folders and select eligible items for Review & Clean.`,
    reason: 'Drive root — expand to select specific folders', tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false,
    displayOnly: false, selectable: false, children,
  });
}

function indexAnalyzerNodes(root: AnalyzerNodeDto): Map<string, AnalyzerNodeDto> {
  const map = new Map<string, AnalyzerNodeDto>();
  const walk = (n: AnalyzerNodeDto) => {
    map.set(n.id, n);
    for (const c of n.children ?? []) walk(c);
  };
  walk(root);
  return map;
}

function selectionNeedsConfirmation(nodes: AnalyzerNodeDto[], scanItems?: ScanItemInfo[]): boolean {
  if (scanItems && scanItems.some((i) => i.risk === 'Unknown' || i.targetId.startsWith('custom-path'))) return true;
  return nodes.some((n) => n.risk === 'Unknown' || !n.targetId);
}

function analyzerBreakdown(selected: AnalyzerNodeDto[]): PreviewTargetBreakdownDto[] {
  const by = new Map<string, { targetId: string; targetName: string; count: bigint; bytes: bigint }>();
  for (const n of selected) {
    const targetId = n.targetId ?? 'custom-path';
    const targetName = targetId === 'custom-path' ? 'Custom Path' : (n.targetId ?? n.name);
    const e = by.get(targetId) ?? { targetId, targetName, count: 0n, bytes: 0n };
    e.count += 1n;
    e.bytes += BigInt(n.sizeBytes);
    by.set(targetId, e);
  }
  return [...by.values()].map((e) => ({ targetId: e.targetId, targetName: e.targetName, selectedCount: e.count.toString(), selectedBytes: e.bytes.toString() }));
}

export function createMockController(): MockController {
  const init = typeof globalThis !== 'undefined' ? (globalThis as { __SWEEPER_MOCK_INIT__?: { elevationStatus?: { isElevated: boolean }; relaunchResult?: { launched: boolean } | { error: ContractError } } }).__SWEEPER_MOCK_INIT__ : undefined;
  let nowMs = Date.now();
  let settingsGeneration = 1;
  let settingsState: Settings = {
    enableTier1: true,
    enableTier2: true,
    enableTier3: false,
    enableElevationTargets: false,
    minAgeHours: 24,
    allowDeleteUnknown: false,
    customPaths: [],
    staleProjectDays: 30,
  };
  let elevationStatus = { isElevated: init?.elevationStatus?.isElevated ?? false };
  let relaunchResult: { launched: boolean } | { error: ContractError } = init?.relaunchResult ?? { launched: true };
  const sessions = new Map<string, SessionState>();
  const analyzerSessions = new Map<string, AnalyzerSessionState>();
  const previews = new Map<string, PreviewState>();
  let scanSeq = 0;
  let analyzerSeq = 0;
  let previewSeq = 0;
  let logSeq = 0;
  const logs: LogEntry[] = [
    {
      id: 'log-1',
      name: '2026-11-13-scan.jsonl',
      modifiedUnixSecs: '1794528000',
      sizeBytes: '2048',
      content:
        '{"timestamp":"2:00 AM","action":"User Temp","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp","size":"1048576","status":"Eligible"}\n' +
        '{"timestamp":"2:00 AM","action":"Crash Dumps & WER","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps","size":"5242880","status":"Eligible"}\n' +
        '{"timestamp":"2:00 AM","action":"npm Package Cache","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache","size":"7340032","status":"Eligible"}\n' +
        '{"timestamp":"2:00 AM","action":"Custom Path","details":"C:\\\\Users\\\\Alice\\\\CustomCleanup","size":"8192","status":"Skipped · < 24h"}\n' +
        '{"record":"outcome","status":"completed","event":"scan","categoriesCount":4,"items":5,"totalBytes":"13639680","timestamp":"2:00 AM","action":"Scan completed","details":"4 categories · 13 MB reclaimable"}',
    },
    {
      id: 'log-2',
      name: '2026-11-12-clean.jsonl',
      modifiedUnixSecs: '1794441600',
      sizeBytes: '512',
      content:
        '{"timestamp":"Yesterday, 2:00 AM","action":"User Temp","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\a.tmp","size":"1048576","status":"Cleaned"}\n' +
        '{"record":"outcome","status":"completed","event":"clean","deletedCount":"1","bytesFreed":"1048576","skippedCount":"0","category":"User Temp"}',
    },
  ];

  function bumpSettingsGeneration() { settingsGeneration += 1; }
  function isExpired(createdAtMs: number) { return nowMs - createdAtMs > 15 * 60 * 1000; }

  function getScanSession(sessionId: string): SessionState {
    const s = sessions.get(sessionId);
    if (!s) err('sessionInvalid', 'Session not found.');
    if (isExpired(s.createdAtMs)) err('sessionInvalid', 'Session expired.');
    if (s.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
    return s;
  }
  function getAnalyzerSession(sessionId: string): AnalyzerSessionState {
    const s = analyzerSessions.get(sessionId);
    if (!s) err('sessionInvalid', 'Session not found.');
    if (isExpired(s.createdAtMs)) err('sessionInvalid', 'Session expired.');
    if (s.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
    return s;
  }
  function getAnySessionMeta(sessionId: string): { settingsGeneration: number; createdAtMs: number } {
    const s = sessions.get(sessionId);
    if (s) {
      if (isExpired(s.createdAtMs)) err('sessionInvalid', 'Session expired.');
      if (s.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
      return { settingsGeneration: s.settingsGeneration, createdAtMs: s.createdAtMs };
    }
    const a = analyzerSessions.get(sessionId);
    if (a) {
      if (isExpired(a.createdAtMs)) err('sessionInvalid', 'Session expired.');
      if (a.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
      return { settingsGeneration: a.settingsGeneration, createdAtMs: a.createdAtMs };
    }
    err('sessionInvalid', 'Session not found.');
  }

  function makeScanResults(tiers: TierDto[] | null): { targets: TargetScanResultDto[]; itemsById: SessionState['itemsById'] } {
    const enabled = new Set(tiers ?? ([
      settingsState.enableTier1 ? 'Tier1' : null,
      settingsState.enableTier2 ? 'Tier2' : null,
      settingsState.enableTier3 ? 'Tier3' : null,
    ].filter(Boolean) as TierDto[]));
    const itemsById: SessionState['itemsById'] = new Map();
    const targets: TargetScanResultDto[] = [];

    const defs: Array<{ tier: TierDto; build: () => ReturnType<typeof buildTarget> }> = [
      { tier: 'Tier1', build: () => buildTarget('t-user-temp', 'User Temp', 'Tier1', 'Safe', 'Temporary files regenerated automatically.', SAMPLE_ITEMS_T1.slice(0, 2)) },
      { tier: 'Tier1', build: () => buildTarget('t-crash', 'Crash Dumps & WER', 'Tier1', 'Safe', 'Local crash reports and error dumps.', SAMPLE_ITEMS_T1.slice(2)) },
      { tier: 'Tier2', build: () => buildTarget('t-npm', 'npm-cache', 'Tier2', 'Safe', 'Package manager download cache, redownloaded as needed.', SAMPLE_ITEMS_T2) },
    ];

    for (const def of defs) {
      if (!enabled.has(def.tier)) continue;
      const { target, items } = def.build();
      targets.push(target);
      for (const it of items) itemsById.set(it.id, it);
    }

    const customTargetId = 'custom-path-1';
    const customItem: ScanItemInfo = {
      id: `${customTargetId}-item-1`,
      targetId: customTargetId,
      path: 'C:\\Users\\Alice\\CustomCleanup\\old.bin',
      size: '8192',
      mtimeUnixSecs: '1700000000',
      ageEligible: true,
      risk: 'Unknown',
      targetName: 'Custom Path (Unknown)',
      description: 'Custom Path selection requires typed confirmation. Unknown items require allow_delete_unknown.',
    };
    targets.push({
      targetId: customTargetId,
      targetName: 'Custom Path (Unknown)',
      tier: 'Tier2',
      risk: 'Unknown',
      resolvedPaths: ['C:\\Users\\Alice\\CustomCleanup'],
      scanned: true,
      statusReason: settingsState.allowDeleteUnknown ? 'Custom Path / Unknown item requires typed confirmation.' : 'Unknown selection is gated by "Allow deleting unknown folders".',
      totalBytes: customItem.size,
      eligibleBytes: customItem.size,
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: [((({ risk: _r, targetName: _n, description: _d, ...rest }) => rest)(customItem))],
    });
    itemsById.set(customItem.id, customItem);

    return { targets, itemsById };
  }

  return {
    async startScan(tiers, onProgress) {
      scanSeq += 1;
      const sessionId = `mock-session-${scanSeq}`;
      const { targets, itemsById } = makeScanResults(tiers);
      const state: SessionState = {
        id: sessionId,
        createdAtMs: nowMs,
        minAgeHours: settingsState.minAgeHours,
        targets,
        itemsById,
        settingsGeneration,
      };
      sessions.set(sessionId, state);
      onProgress?.({ kind: 'started', session_id: sessionId });
      onProgress?.({ kind: 'completed', session_id: sessionId });
      return { sessionId };
    },

    async getScanResults(sessionId) {
      const s = getScanSession(sessionId);
      return {
        sessionId: s.id,
        startedUnixSecs: Math.floor(s.createdAtMs / 1000).toString(),
        minAgeHours: s.minAgeHours,
        targets: s.targets,
      };
    },

    async startAnalyzer(root, onProgress) {
      analyzerSeq += 1;
      const sessionId = `mock-analyzer-${analyzerSeq}`;
      const rootPath = (root && root.trim()) || 'C:\\';
      onProgress?.({ sessionId, scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: rootPath });
      const tree = buildAnalyzerTree(rootPath, settingsState.allowDeleteUnknown);
      const result: StartAnalyzerResult = {
        sessionId,
        rootPath,
        totalBytes: tree.sizeBytes,
        totalItems: tree.itemCount,
        scannedDirs: '42',
        scannedFiles: String(indexAnalyzerNodes(tree).size),
        cancelled: false,
        warnings: [],
        root: tree,
      };
      onProgress?.({ sessionId, scannedDirs: '42', scannedFiles: result.scannedFiles, scannedBytes: result.totalBytes, currentPath: null });
      analyzerSessions.set(sessionId, {
        id: sessionId,
        createdAtMs: nowMs,
        rootPath,
        result,
        nodesById: indexAnalyzerNodes(tree),
        settingsGeneration,
      });
      return result;
    },

    async previewClean(sessionId, itemIds, confirmations) {
      if (!itemIds.length) err('invalidInput', 'No items selected.');
      if (sessions.has(sessionId)) {
        const s = getScanSession(sessionId);
        let unknownCount = 0;
        const selected: Array<{ item: ScanItemInfo; targetId: string }> = [];
        for (const id of itemIds) {
          const item = s.itemsById.get(id);
          if (!item) { unknownCount += 1; continue; }
          if (item.risk === 'Protected') err('invalidInput', 'Protected items cannot be selected.');
          if (!item.ageEligible) err('invalidInput', 'Ineligible items cannot be selected.');
          if (item.risk === 'Unknown' && !settingsState.allowDeleteUnknown) {
            err('invalidInput', 'Unknown items are gated by allow_delete_unknown.');
          }
          selected.push({ item, targetId: item.targetId });
        }
        if (unknownCount > 0) err('unknownItem', String(unknownCount));
        const needsConfirm = selected.some((x) => x.item.risk === 'Unknown' || x.item.targetId.startsWith('custom-path'));
        if (needsConfirm) {
          const typed = confirmations?.typedConfirmation ?? null;
          if (typed !== 'DELETE') err('invalidInput', 'Typed confirmation required: enter DELETE to continue.');
        }
        const byTarget = new Map<string, { targetId: string; targetName: string; count: bigint; bytes: bigint }>();
        let totalBytes = 0n;
        for (const { item } of selected) {
          const entry = byTarget.get(item.targetId) ?? { targetId: item.targetId, targetName: item.targetName, count: 0n, bytes: 0n };
          entry.count += 1n;
          entry.bytes += BigInt(item.size);
          byTarget.set(item.targetId, entry);
          totalBytes += BigInt(item.size);
        }
        const breakdown: PreviewTargetBreakdownDto[] = [...byTarget.values()].map((e) => ({
          targetId: e.targetId, targetName: e.targetName, selectedCount: e.count.toString(), selectedBytes: e.bytes.toString(),
        }));
        previewSeq += 1;
        const token = `mock-preview-${previewSeq}`;
        previews.set(token, { token, sessionId, createdAtMs: nowMs, used: false, itemIds: [...itemIds], settingsGeneration, previewEstimatedBytes: totalBytes.toString() });
        return { previewToken: token, breakdown, totalSelectedCount: BigInt(selected.length).toString(), totalSelectedBytes: totalBytes.toString() };
      }

      if (analyzerSessions.has(sessionId)) {
        const s = getAnalyzerSession(sessionId);
        let unknownCount = 0;
        const selected: AnalyzerNodeDto[] = [];
        for (const id of itemIds) {
          const node = s.nodesById.get(id);
          if (!node) { unknownCount += 1; continue; }
          if (node.risk === 'Unknown' && !settingsState.allowDeleteUnknown) err('invalidInput', 'Unknown items are gated by allow_delete_unknown.');
          if (!node.selectable || node.displayOnly || node.risk === 'Protected' || node.isLinkPoint || node.isCloudPlaceholder) {
            err('invalidInput', 'Selected analyzer node is not eligible for Review & Clean.');
          }
          selected.push(node);
        }
        if (unknownCount > 0) err('unknownItem', String(unknownCount));
        if (selectionNeedsConfirmation(selected)) {
          const typed = confirmations?.typedConfirmation ?? null;
          if (typed !== 'DELETE') err('invalidInput', 'Typed confirmation required: enter DELETE to continue.');
        }
        const breakdown = analyzerBreakdown(selected);
        const totalBytes = selected.reduce((a, n) => a + BigInt(n.sizeBytes), 0n);
        previewSeq += 1;
        const token = `mock-preview-${previewSeq}`;
        previews.set(token, { token, sessionId, createdAtMs: nowMs, used: false, itemIds: [...itemIds], settingsGeneration, previewEstimatedBytes: totalBytes.toString() });
        return { previewToken: token, breakdown, totalSelectedCount: BigInt(selected.length).toString(), totalSelectedBytes: totalBytes.toString() };
      }

      err('sessionInvalid', 'Session not found.');
    },

    async executeClean(previewToken, onProgress) {
      const p = previews.get(previewToken);
      if (!p) err('previewTokenInvalid', 'Preview token not found.');
      if (p.used) err('previewTokenInvalid', 'Preview token already used.');
      if (nowMs - p.createdAtMs > 10 * 60 * 1000) err('previewTokenInvalid', 'Preview token expired.');
      getAnySessionMeta(p.sessionId);
      if (p.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
      p.used = true;

      let deletedBytes = 0n;
      let deletedCount = 0n;
      let skippedCount = 0n;
      const reasons: Record<string, bigint> = {};
      const total = BigInt(p.itemIds.length);
      let completed = 0n;
      const scanSession = sessions.get(p.sessionId);
      const analyzerSession = analyzerSessions.get(p.sessionId);
      for (const id of p.itemIds) {
        let targetId = 'unknown';
        let ok = false;
        let bytes = 0n;
        if (scanSession) {
          const item = scanSession.itemsById.get(id);
          targetId = item?.targetId ?? 'unknown';
          if (item && item.ageEligible && item.risk !== 'Protected' && !(item.risk === 'Unknown' && !settingsState.allowDeleteUnknown)) {
            ok = true; bytes = BigInt(item.size);
            scanSession.itemsById.delete(id);
            for (const t of scanSession.targets) t.items = t.items.filter((i) => i.id !== id);
          }
        } else if (analyzerSession) {
          const node = analyzerSession.nodesById.get(id);
          targetId = node?.targetId ?? 'custom-path';
          if (node && node.selectable && !node.displayOnly && node.risk !== 'Protected' && !node.isLinkPoint && !node.isCloudPlaceholder && !(node.risk === 'Unknown' && !settingsState.allowDeleteUnknown)) {
            ok = true; bytes = BigInt(node.sizeBytes);
          }
        }
        if (ok) {
          deletedBytes += bytes;
          deletedCount += 1n;
        } else {
          skippedCount += 1n;
          reasons['Skipped: not eligible'] = (reasons['Skipped: not eligible'] ?? 0n) + 1n;
        }
        completed += 1n;
        onProgress?.({ previewToken, targetId, completedItems: completed.toString(), totalItems: total.toString(), bytesFreedSoFar: deletedBytes.toString(), status: ok ? 'deleted' : 'Skipped' });
      }
      const skippedReasons: SkippedReasonDto[] = Object.entries(reasons).map(([reason, count]) => ({ reason, count: count.toString() }));
      logSeq += 1;
      const ts = Math.floor(nowMs / 1000).toString();
      const logId = `mock-clean-${logSeq}`;
      const logName = `clean-${logId}.jsonl`;
      const content = [
        ...p.itemIds.map((id) => JSON.stringify({ record: 'intent', target_id: id, timestamp: ts })),
        JSON.stringify({ record: 'outcome', status: 'completed', deletedCount: deletedCount.toString(), bytesFreed: deletedBytes.toString(), timestamp: ts }),
      ].join('\n') + '\n';
      logs.unshift({ id: logId, name: logName, modifiedUnixSecs: ts, sizeBytes: String(content.length), content });
      return { bytesFreed: deletedBytes.toString(), deletedCount: deletedCount.toString(), skippedCount: skippedCount.toString(), skippedReasons, aborted: false, abortReason: null, logId };
    },

    async explainItem(itemId: string) {
      for (const s of sessions.values()) {
        const item = s.itemsById.get(itemId);
        if (item) return { itemId, title: item.targetName, description: item.description, risk: item.risk };
      }
      for (const s of analyzerSessions.values()) {
        const node = s.nodesById.get(itemId);
        if (node) return { itemId, title: node.name || node.path, description: node.description, risk: node.risk };
      }
      err('notFound', 'Item not found.');
    },

    async getSettings() {
      return { ...settingsState, customPaths: [...settingsState.customPaths] };
    },

    async updateSettings(settings) {
      settingsState = {
        ...settings,
        enableElevationTargets: settings.enableElevationTargets ?? false,
        customPaths: [...(settings.customPaths ?? [])],
      };
      bumpSettingsGeneration();
      return { ...settingsState, customPaths: [...settingsState.customPaths] };
    },

    async getElevationStatus() {
      return { ...elevationStatus };
    },

    async relaunchElevated() {
      if ('error' in relaunchResult) throw relaunchResult.error;
      return { ...relaunchResult };
    },

    async validateCustomPath(path) {
      const trimmed = path.trim();
      if (!trimmed) return { path: trimmed, allowed: false, reason: 'Path is required.' };
      if (!/^[A-Za-z]:\\/.test(trimmed)) return { path: trimmed, allowed: false, reason: 'Path must be absolute.' };
      if (trimmed.toLowerCase().startsWith('c:\\windows')) return { path: trimmed, allowed: false, reason: 'Protected Path (no fallback to parent).' };
      if (trimmed.toLowerCase().includes('\\missing')) return { path: trimmed, allowed: false, reason: 'Path does not exist (no fallback to parent).' };
      if ((trimmed.match(/\\/g) || []).length < 2) return { path: trimmed, allowed: false, reason: 'Path must have depth >= 3 segments.' };
      return { path: trimmed, allowed: true, reason: 'ok' };
    },

    async listLogs() {
      return logs.map((l) => ({ id: l.id, name: l.name, modifiedUnixSecs: l.modifiedUnixSecs, sizeBytes: l.sizeBytes }));
    },

    async readLog(logId: string) {
      const log = logs.find((l) => l.id === logId);
      if (!log) err('notFound', 'Log not found.');
      return { id: log.id, content: log.content };
    },

    advanceTimeMs(ms) { nowMs += ms; },
    invalidateSettings() { bumpSettingsGeneration(); },
    setElevationStatus(status) { elevationStatus = { ...status }; },
    setRelaunchResult(result) { relaunchResult = 'error' in result ? result : { ...result }; },
    reset() {
      sessions.clear();
      analyzerSessions.clear();
      previews.clear();
      settingsGeneration = 1;
      settingsState = { enableTier1: true, enableTier2: true, enableTier3: false, enableElevationTargets: false, minAgeHours: 24, allowDeleteUnknown: false, customPaths: [], staleProjectDays: 30 };
      elevationStatus = { isElevated: false };
      relaunchResult = { launched: true };
      scanSeq = 0; analyzerSeq = 0; previewSeq = 0; logSeq = 0; nowMs = Date.now();
      logs.length = 0;
      logs.push(
        {
          id: 'log-1',
          name: '2026-11-13-scan.jsonl',
          modifiedUnixSecs: '1794528000',
          sizeBytes: '2048',
          content:
            '{"timestamp":"2:00 AM","action":"User Temp","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp","size":"1048576","status":"Eligible"}\n' +
            '{"timestamp":"2:00 AM","action":"Crash Dumps & WER","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps","size":"5242880","status":"Eligible"}\n' +
            '{"timestamp":"2:00 AM","action":"npm Package Cache","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache","size":"7340032","status":"Eligible"}\n' +
            '{"timestamp":"2:00 AM","action":"Custom Path","details":"C:\\\\Users\\\\Alice\\\\CustomCleanup","size":"8192","status":"Skipped · < 24h"}\n' +
            '{"record":"outcome","status":"completed","event":"scan","categoriesCount":4,"items":5,"totalBytes":"13639680","timestamp":"2:00 AM","action":"Scan completed","details":"4 categories · 13 MB reclaimable"}',
        },
        {
          id: 'log-2',
          name: '2026-11-12-clean.jsonl',
          modifiedUnixSecs: '1794441600',
          sizeBytes: '512',
          content:
            '{"timestamp":"Yesterday, 2:00 AM","action":"User Temp","details":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\a.tmp","size":"1048576","status":"Cleaned"}\n' +
            '{"record":"outcome","status":"completed","event":"clean","deletedCount":"1","bytesFreed":"1048576","skippedCount":"0","category":"User Temp"}',
        },
      );
    },
  };
}

export const mockController = createMockController();
