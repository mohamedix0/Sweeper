import type {
  ContractError,
  PreviewTargetBreakdownDto,
  RiskDto,
  ScanItemDto,
  TargetScanResultDto,
  TierDto,
} from '../contract/bindings';

type ScanProgress =
  | { kind: 'started'; session_id: string }
  | { kind: 'completed'; session_id: string }
  | { kind: 'failed'; session_id: string; error: ContractError };

export type ScanResults = {
  sessionId: string;
  startedUnixSecs: string;
  minAgeHours: number;
  targets: TargetScanResultDto[];
};

export type PreviewCleanResult = {
  previewToken: string;
  breakdown: PreviewTargetBreakdownDto[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
};

export type ExplainResult = { itemId: string; title: string; description: string; risk: RiskDto };

export type SettingsDto = {
  enableTier1: boolean;
  enableTier2: boolean;
  enableTier3: boolean;
  minAgeHours: number;
  allowDeleteUnknown: boolean;
};

export type LogMeta = { id: string; name: string; modifiedUnixSecs: string | null; sizeBytes: string | null };
export type LogContent = { id: string; content: string };

type SessionState = {
  id: string;
  createdAtMs: number;
  minAgeHours: number;
  targets: TargetScanResultDto[];
  itemsById: Map<string, ScanItemDto & { targetId: string; risk: RiskDto; targetName: string; description: string }>;
  settingsGeneration: number;
};

type PreviewState = {
  token: string;
  sessionId: string;
  createdAtMs: number;
  used: boolean;
  itemIds: string[];
  settingsGeneration: number;
};

export type MockController = {
  startScan: (tiers: TierDto[] | null, onProgress?: (e: ScanProgress) => void) => Promise<{ sessionId: string }>;
  getScanResults: (sessionId: string) => Promise<ScanResults>;
  previewClean: (
    sessionId: string,
    itemIds: string[],
    confirmations: { typedConfirmation: string | null } | null,
  ) => Promise<PreviewCleanResult>;
  explainItem: (itemId: string) => Promise<ExplainResult>;
  getSettings: () => Promise<SettingsDto>;
  listLogs: () => Promise<LogMeta[]>;
  readLog: (logId: string) => Promise<LogContent>;
  // test helpers
  advanceTimeMs: (ms: number) => void;
  invalidateSettings: () => void;
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
  { targetId: 't-npm', path: 'C:\\Users\\Alice\\AppData\\Local\\npm-cache\\_cacache\\index-v5', size: '7340032', mtimeUnixSecs: '1705000000', ageEligible: true },
];

function buildTarget(
  targetId: string,
  targetName: string,
  tier: TierDto,
  risk: RiskDto,
  description: string,
  rawItems: Array<Omit<ScanItemDto, 'id'>>,
): { target: TargetScanResultDto; items: Array<ScanItemDto & { targetId: string; risk: RiskDto; targetName: string; description: string }> } {
  const items = rawItems.map((it, i) => ({
    ...it,
    id: `${targetId}-item-${i + 1}`,
    risk,
    targetName,
    description,
  }));
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

export function createMockController(): MockController {
  let nowMs = Date.now();
  let settingsGeneration = 1;
  const sessions = new Map<string, SessionState>();
  const previews = new Map<string, PreviewState>();
  let scanSeq = 0;
  let previewSeq = 0;

  function getSession(sessionId: string): SessionState {
    const s = sessions.get(sessionId);
    if (!s) err('sessionInvalid', 'Session not found.');
    if (nowMs - s.createdAtMs > 15 * 60 * 1000) err('sessionInvalid', 'Session expired.');
    if (s.settingsGeneration !== settingsGeneration) err('sessionInvalid', 'Session invalidated by settings change.');
    return s;
  }

  function makeScanResults(tiers: TierDto[] | null): { targets: TargetScanResultDto[]; itemsById: SessionState['itemsById'] } {
    const enabled = new Set(tiers ?? ['Tier1', 'Tier2']);
    const itemsById: SessionState['itemsById'] = new Map();
    const targets: TargetScanResultDto[] = [];

    const defs: Array<{ tier: TierDto; build: () => ReturnType<typeof buildTarget> }> = [
      {
        tier: 'Tier1',
        build: () =>
          buildTarget('t-user-temp', 'User Temp', 'Tier1', 'Safe', 'Temporary files regenerated automatically.', SAMPLE_ITEMS_T1.slice(0, 2)),
      },
      {
        tier: 'Tier1',
        build: () =>
          buildTarget('t-crash', 'Crash Dumps & WER', 'Tier1', 'Safe', 'Local crash reports and error dumps.', SAMPLE_ITEMS_T1.slice(2)),
      },
      {
        tier: 'Tier2',
        build: () =>
          buildTarget('t-npm', 'npm-cache', 'Tier2', 'Safe', 'Package manager download cache, redownloaded as needed.', SAMPLE_ITEMS_T2),
      },
    ];

    // include a protected display-only target and an unknown example for UI behavior
    const protectedTarget: TargetScanResultDto = {
      targetId: 't-protected-system32',
      targetName: 'System32 (Protected)',
      tier: 'Tier1',
      risk: 'Protected',
      resolvedPaths: ['C:\\Windows\\System32'],
      scanned: true,
      statusReason: 'Protected Path — never selectable or removable.',
      totalBytes: '4096',
      eligibleBytes: '0',
      fileCount: '1',
      eligibleCount: '0',
      skippedCount: '1',
      items: [
        { id: 't-protected-system32-item-1', targetId: 't-protected-system32', path: 'C:\\Windows\\System32\\kernel32.dll', size: '4096', mtimeUnixSecs: '1600000000', ageEligible: false },
      ],
    };

    for (const def of defs) {
      if (!enabled.has(def.tier)) continue;
      const { target, items } = def.build();
      targets.push(target);
      for (const it of items) itemsById.set(it.id, it);
    }
    targets.push(protectedTarget);
    itemsById.set('t-protected-system32-item-1', {
      id: 't-protected-system32-item-1',
      targetId: 't-protected-system32',
      path: 'C:\\Windows\\System32\\kernel32.dll',
      size: '4096',
      mtimeUnixSecs: '1600000000',
      ageEligible: false,
      risk: 'Protected',
      targetName: 'System32 (Protected)',
      description: 'Protected Path. Displayed for context only and never removable.',
    });

    const unknownItems: Array<ScanItemDto & { targetId: string; risk: RiskDto; targetName: string; description: string }> = [
      {
        id: 't-unknown-item-1',
        targetId: 't-unknown-folder',
        path: 'C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache\\old.bin',
        size: '8192',
        mtimeUnixSecs: '1700000000',
        ageEligible: true,
        risk: 'Unknown',
        targetName: 'Unknown Folder (Custom/Unrecognized)',
        description: 'Unknown location. Requires typed confirmation before Preview/Clean flows in advanced scenarios.',
      },
    ];
    targets.push({
      targetId: 't-unknown-folder',
      targetName: 'Unknown Folder (Custom/Unrecognized)',
      tier: 'Tier2',
      risk: 'Unknown',
      resolvedPaths: ['C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache'],
      scanned: true,
      statusReason: 'Unknown item shown for review. Selection requires typed confirmation in Preview.',
      totalBytes: '8192',
      eligibleBytes: '8192',
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: unknownItems.map(({ risk: _r, targetName: _n, description: _d, ...rest }) => rest),
    });
    for (const it of unknownItems) itemsById.set(it.id, it);

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
        minAgeHours: 24,
        targets,
        itemsById,
        settingsGeneration,
      };
      sessions.set(sessionId, state);
      onProgress?.({ kind: 'started', session_id: sessionId });
      // deterministic: complete immediately (UI can animate if desired)
      onProgress?.({ kind: 'completed', session_id: sessionId });
      return { sessionId };
    },

    async getScanResults(sessionId) {
      const s = getSession(sessionId);
      return {
        sessionId: s.id,
        startedUnixSecs: Math.floor(s.createdAtMs / 1000).toString(),
        minAgeHours: s.minAgeHours,
        targets: s.targets,
      };
    },

    async previewClean(sessionId, itemIds, confirmations) {
      const s = getSession(sessionId);
      if (!itemIds.length) err('invalidInput', 'No items selected.');
      let unknown = 0;
      const selected = [] as Array<{ item: ScanItemDto & { risk: RiskDto; targetName: string }; targetId: string }>;
      for (const id of itemIds) {
        const item = s.itemsById.get(id);
        if (!item) {
          unknown += 1;
          continue;
        }
        if (item.risk === 'Protected') err('invalidInput', 'Protected items cannot be selected.');
        if (!item.ageEligible) err('invalidInput', 'Ineligible items cannot be selected.');
        selected.push({ item, targetId: item.targetId });
      }
      if (unknown > 0) err('unknownItem', String(unknown));
      const needsConfirm = selected.some((s) => s.item.risk === 'Unknown' || s.item.targetId.startsWith('custom-'));
      if (needsConfirm) {
        const typed = confirmations?.typedConfirmation?.trim();
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
        targetId: e.targetId,
        targetName: e.targetName,
        selectedCount: e.count.toString(),
        selectedBytes: e.bytes.toString(),
      }));
      previewSeq += 1;
      const token = `mock-preview-${previewSeq}`;
      previews.set(token, {
        token,
        sessionId,
        createdAtMs: nowMs,
        used: false,
        itemIds: [...itemIds],
        settingsGeneration,
      });
      return {
        previewToken: token,
        breakdown,
        totalSelectedCount: BigInt(selected.length).toString(),
        totalSelectedBytes: totalBytes.toString(),
      };
    },

    async explainItem(itemId: string) {
      for (const s of sessions.values()) {
        const item = s.itemsById.get(itemId);
        if (item) {
          return { itemId, title: item.targetName, description: item.description, risk: item.risk };
        }
      }
      err('notFound', 'Item not found.');
    },

    async getSettings() {
      return {
        enableTier1: true,
        enableTier2: false,
        enableTier3: false,
        minAgeHours: 24,
        allowDeleteUnknown: false,
      };
    },

    async listLogs() {
      return [
        { id: 'log-1', name: '2026-11-13-scan.jsonl', modifiedUnixSecs: '1794528000', sizeBytes: '2048' },
        { id: 'log-2', name: '2026-11-12-clean.jsonl', modifiedUnixSecs: '1794441600', sizeBytes: '512' },
      ];
    },

    async readLog(logId: string) {
      if (logId === 'log-1') {
        return { id: logId, content: '{"event":"scan","status":"completed","targets":3}\n{"event":"preview","items":2}' };
      }
      if (logId === 'log-2') {
        return { id: logId, content: '{"event":"clean","status":"completed","deleted":1}' };
      }
      err('notFound', 'Log not found.');
    },

    advanceTimeMs(ms) {
      nowMs += ms;
    },
    invalidateSettings() {
      settingsGeneration += 1;
    },
    reset() {
      sessions.clear();
      previews.clear();
      settingsGeneration = 1;
      scanSeq = 0;
      previewSeq = 0;
      nowMs = Date.now();
    },
  };
}

export const mockController = createMockController();
