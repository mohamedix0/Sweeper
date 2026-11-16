import type { AnalyzerNodeDto, RiskDto, ScanItemDto, TargetScanResultDto, TierDto } from '../../contract/bindings';
import type { Settings } from '../client';

export type ScanItemInfo = ScanItemDto & { targetId: string; risk: RiskDto; targetName: string; description: string };

const SAMPLE_ITEMS_T1: Array<Omit<ScanItemDto, 'id'>> = [
  { targetId: 't-user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp', size: '1048576', mtimeUnixSecs: '1710000000', ageEligible: true },
  { targetId: 't-user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\recent.log', size: '2048', mtimeUnixSecs: '1899999999', ageEligible: false },
  { targetId: 't-crash', path: 'C:\\Users\\Alice\\AppData\\Local\\CrashDumps\\app.dmp', size: '5242880', mtimeUnixSecs: '1700000000', ageEligible: true },
];
const SAMPLE_ITEMS_T2: Array<Omit<ScanItemDto, 'id'>> = [
  { targetId: 't-npm', path: 'C:\\Users\\Alice\\AppData\\Local\\npm-cache\\package-cache-tarballs', size: '7340032', mtimeUnixSecs: '1705000000', ageEligible: true },
];

function itemStats(items: ScanItemInfo[]) {
  let total = 0n;
  let eligible = 0n;
  let eligibleCount = 0n;
  let skipped = 0n;
  for (const it of items) {
    const sz = BigInt(it.size);
    total += sz;
    if (it.ageEligible) { eligible += sz; eligibleCount += 1n; }
    else { skipped += 1n; }
  }
  return { total, eligible, eligibleCount, skipped };
}

function toScanItem(item: ScanItemInfo): ScanItemDto {
  const { risk: _r, targetName: _n, description: _d, ...rest } = item;
  return rest;
}

export function buildTarget(input: { targetId: string; targetName: string; tier: TierDto; risk: RiskDto; description: string; rawItems: Array<Omit<ScanItemDto, 'id' | 'targetId'> & { targetId?: string }> }) {
  const items = input.rawItems.map((it, i) => ({ ...it, id: input.targetId + '-item-' + String(i + 1), risk: input.risk, targetName: input.targetName, description: input.description })) as ScanItemInfo[];
  const s = itemStats(items);
  const firstPath = items.length ? items[0].path.replace(/\\[^\\]+$/, '') : '';
  const resolvedPaths = firstPath ? [firstPath] : [];
  const target: TargetScanResultDto = {
    targetId: input.targetId, targetName: input.targetName, tier: input.tier, risk: input.risk,
    resolvedPaths, scanned: true, statusReason: null,
    totalBytes: s.total.toString(), eligibleBytes: s.eligible.toString(),
    fileCount: BigInt(items.length).toString(), eligibleCount: s.eligibleCount.toString(), skippedCount: s.skipped.toString(),
    items: items.map(toScanItem),
  };
  return { target, items };
}

export function makeNode(input: Omit<AnalyzerNodeDto, 'children'> & { children?: AnalyzerNodeDto[] }): AnalyzerNodeDto {
  return { ...input, children: input.children ?? [] };
}

type NodeOverrides = Partial<Omit<AnalyzerNodeDto, 'id' | 'name' | 'path' | 'children'>> & { children?: AnalyzerNodeDto[] };

function nodeDefaults(input: { id: string; name: string; path: string }): Omit<AnalyzerNodeDto, 'children'> {
  return {
    id: input.id, name: input.name, path: input.path, isDir: false, sizeBytes: '0', itemCount: '1',
    risk: 'Safe', description: '', reason: null, tier: null, targetId: null,
    isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
  };
}

function makeAnalyzerNode(input: { id: string; name: string; path: string } & NodeOverrides): AnalyzerNodeDto {
  const base = nodeDefaults({ id: input.id, name: input.name, path: input.path });
  const { id: _id, name: _name, path: _path, children, ...overrides } = input;
  return makeNode({ ...base, ...overrides, children: children ?? [] });
}

function sumSizes(nodes: AnalyzerNodeDto[]): bigint {
  return nodes.reduce((a, n) => a + BigInt(n.sizeBytes), 0n);
}

function sumItems(nodes: AnalyzerNodeDto[]): bigint {
  return nodes.reduce((a, n) => a + BigInt(n.itemCount), 0n);
}

export function buildDeepBranch(input: { base: string }): AnalyzerNodeDto {
  const root = input.base + '\\Users\\Alice\\AppData\\Local\\Packages\\DeepVendor';
  let leaf: AnalyzerNodeDto = makeAnalyzerNode({ id: 'an-deep-leaf', name: 'cache.bin', path: root + '\\layer30\\cache.bin', sizeBytes: '2048', risk: 'Safe', description: 'Deep nested cache file (Safe).', tier: 'Tier2', targetId: 'mock-deep-safe', children: [] });
  for (let i = 30; i >= 1; i--) {
    leaf = makeAnalyzerNode({ id: 'an-deep-dir-' + String(i), name: 'layer' + String(i), path: root + '\\layer' + String(i), isDir: true, sizeBytes: String(2048 + i), itemCount: String(31 - i), description: 'Deep nested directory for virtualization testing.', tier: 'Tier2', targetId: 'mock-deep-safe', children: [leaf] });
  }
  return makeAnalyzerNode({ id: 'an-deep-root', name: 'DeepVendor', path: root, isDir: true, sizeBytes: '4096', itemCount: '31', description: 'Deep nested structure (virtualization demo).', tier: 'Tier2', targetId: 'mock-deep-safe', children: [leaf] });
}

export function buildBulkFiles(input: { base: string }): AnalyzerNodeDto[] {
  const files: AnalyzerNodeDto[] = [];
  for (let i = 1; i <= 40; i++) {
    files.push(makeAnalyzerNode({ id: 'an-bulk-file-' + String(i), name: 'temp-' + String(i) + '.tmp', path: input.base + '\\Users\\Alice\\AppData\\Local\\Temp\\bulk\\temp-' + String(i) + '.tmp', sizeBytes: String(1024 * i), description: 'Temporary file (Safe).', tier: 'Tier1', targetId: 'mock-temp' }));
  }
  return files;
}

function protectedGit(input: { base: string }) {
  const root = input.base + '\\Dev\\repo\\.git';
  const child = makeAnalyzerNode({ id: 'an-protected-git-obj', name: 'objects', path: root + '\\objects', isDir: true, sizeBytes: '2048', itemCount: '4', risk: 'Protected', description: 'Protected git internals.', reason: 'Protected Path', displayOnly: true, selectable: false });
  return makeAnalyzerNode({ id: 'an-protected-git', name: '.git', path: root, isDir: true, sizeBytes: '4096', itemCount: '12', risk: 'Protected', description: 'Protected repository metadata (.git).', reason: 'Protected Path', displayOnly: true, selectable: false, children: [child] });
}

function specialNodes(input: { base: string }) {
  const linkPoint = makeAnalyzerNode({ id: 'an-link-point', name: 'OneDriveLink', path: input.base + '\\Users\\Alice\\OneDriveLink', isDir: true, itemCount: '0', risk: 'Caution', description: 'Link Point (reparse point). Never followed.', reason: 'Link Point — not traversed', isLinkPoint: true, displayOnly: true, selectable: false });
  const vhdx = makeAnalyzerNode({ id: 'an-vhdx', name: 'dev-machine.vhdx', path: input.base + '\\Users\\Alice\\Virtual Machines\\dev-machine.vhdx', sizeBytes: '1073741824', risk: 'Caution', description: 'Virtual disk image (.vhdx). Display-only by policy.', reason: '.vhdx display-only', displayOnly: true, selectable: false });
  const cloud = makeAnalyzerNode({ id: 'an-cloud', name: 'CloudPlaceholder.txt', path: input.base + '\\Users\\Alice\\OneDrive\\CloudPlaceholder.txt', itemCount: '1', risk: 'Caution', description: 'Cloud placeholder file. Skipped by policy.', reason: 'Cloud placeholder — skipped', isCloudPlaceholder: true, displayOnly: true, selectable: false });
  return [linkPoint, vhdx, cloud];
}

function unknownFolder(input: { base: string; allowUnknown: boolean }) {
  const reason = input.allowUnknown ? 'Unknown (selectable when explicitly allowed)' : 'Unknown items are not selectable until allowed in Settings';
  const file = makeAnalyzerNode({ id: 'an-unknown-file', name: 'old.bin', path: input.base + '\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache\\old.bin', sizeBytes: '8192', risk: 'Unknown', description: 'Unknown cached data file.', reason, selectable: input.allowUnknown });
  return makeAnalyzerNode({ id: 'an-unknown-folder', name: 'MysteryCache', path: input.base + '\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache', isDir: true, sizeBytes: '8192', itemCount: '2', risk: 'Unknown', description: 'Unknown folder not in Knowledge Base. May require confirmation/policy to remove.', reason, selectable: input.allowUnknown, children: [file] });
}

function cautionFolder(input: { base: string }) {
  const log1 = makeAnalyzerNode({ id: 'an-caution-log1', name: 'app.log', path: input.base + '\\ProgramData\\Vendor\\Logs\\app.log', sizeBytes: '131072', risk: 'Caution', description: 'Application log file.', tier: 'Tier2', targetId: 'mock-caution-logs' });
  const log2 = makeAnalyzerNode({ id: 'an-caution-log2', name: 'service.log', path: input.base + '\\ProgramData\\Vendor\\Logs\\service.log', sizeBytes: '131072', risk: 'Caution', description: 'Service log file.', tier: 'Tier2', targetId: 'mock-caution-logs' });
  return makeAnalyzerNode({ id: 'an-caution-logs', name: 'Logs', path: input.base + '\\ProgramData\\Vendor\\Logs', isDir: true, sizeBytes: '262144', itemCount: '8', risk: 'Caution', description: 'Vendor logs (Caution). Review before removal.', tier: 'Tier2', targetId: 'mock-caution-logs', children: [log1, log2] });
}

function safeTemp(input: { base: string }) {
  const manyFiles = buildBulkFiles({ base: input.base });
  const bulk = makeAnalyzerNode({ id: 'an-safe-temp-bulk', name: 'bulk', path: input.base + '\\Users\\Alice\\AppData\\Local\\Temp\\bulk', isDir: true, sizeBytes: sumSizes(manyFiles).toString(), itemCount: String(manyFiles.length), description: 'Bulk temporary files (Safe).', tier: 'Tier1', targetId: 'mock-temp', children: manyFiles });
  const a = makeAnalyzerNode({ id: 'an-safe-temp-a', name: 'a.tmp', path: input.base + '\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp', sizeBytes: '1048576', description: 'Temporary file.', tier: 'Tier1', targetId: 'mock-temp' });
  const recent = makeAnalyzerNode({ id: 'an-safe-temp-recent', name: 'recent.log', path: input.base + '\\Users\\Alice\\AppData\\Local\\Temp\\recent.log', sizeBytes: '2048', description: 'Recent temp log (not age-eligible in scan, but analyzer node shown for selection policy demo).', reason: 'Recent (demo)', tier: 'Tier1', targetId: 'mock-temp' });
  const kids = [a, recent, bulk];
  return makeAnalyzerNode({ id: 'an-safe-temp', name: 'Temp', path: input.base + '\\Users\\Alice\\AppData\\Local\\Temp', isDir: true, sizeBytes: sumSizes(kids).toString(), itemCount: String(2 + manyFiles.length), description: 'User temporary files (Safe).', tier: 'Tier1', targetId: 'mock-temp', children: kids });
}

export function buildAnalyzerTree(input: { rootPath: string; allowUnknown: boolean }): AnalyzerNodeDto {
  const base = input.rootPath.replace(/[\\/]+$/, '');
  const children = [safeTemp({ base }), cautionFolder({ base }), unknownFolder({ base, allowUnknown: input.allowUnknown }), protectedGit({ base }), ...specialNodes({ base }), buildDeepBranch({ base })];
  return makeAnalyzerNode({ id: 'an-root', name: base || input.rootPath, path: input.rootPath, isDir: true, sizeBytes: sumSizes(children).toString(), itemCount: sumItems(children).toString(), risk: 'Caution', description: 'Analyzer root for ' + input.rootPath + '. Expand to review folders and select eligible items for Review & Clean.', reason: 'Drive root — expand to select specific folders', displayOnly: false, selectable: false, children });
}

export function indexAnalyzerNodes(root: AnalyzerNodeDto): Map<string, AnalyzerNodeDto> {
  const map = new Map<string, AnalyzerNodeDto>();
  const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
  walk(root);
  return map;
}

export function defaultSeedLogs(input: { nowMs: number } = { nowMs: Date.now() }) {
  const nowMs = input.nowMs;
  return [
    { id: 'log-1', name: '2026-11-13-scan.jsonl', modifiedUnixSecs: String(Math.floor(nowMs / 1000) - 3 * 24 * 60 * 60), sizeBytes: '3072', content: '{"timestamp":"2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp","size":"1048576","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"Crash Dumps & WER","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps","size":"5242880","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"npm Package Cache","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache","size":"7340032","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\recent.log","size":"2048","status":"Skipped · < 24h","kind":"File"}\n{"timestamp":"2:00 AM","category":"Custom Path (Unknown)","path":"C:\\\\Users\\\\Alice\\\\CustomCleanup\\\\old.bin","size":"8192","status":"Skipped · Unverified","kind":"File"}\n{"record":"outcome","status":"completed","event":"scan","categoriesCount":4,"items":5,"totalBytes":"13639680","timestamp":"2:00 AM"}' },
    { id: 'log-2', name: '2026-11-12-clean.jsonl', modifiedUnixSecs: String(Math.floor(nowMs / 1000) - 4 * 24 * 60 * 60), sizeBytes: '1024', content: '{"timestamp":"Yesterday, 2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\a.tmp","size":"1048576","status":"Cleaned"}\n{"timestamp":"Yesterday, 2:00 AM","category":"Crash Dumps & WER","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps\\\\app.dmp","size":"5242880","status":"Cleaned"}\n{"timestamp":"Yesterday, 2:00 AM","category":"npm Package Cache","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache\\\\index-v5","size":"7340032","status":"Cleaned"}\n{"record":"outcome","status":"completed","event":"clean","deletedCount":"3","bytesFreed":"13631488","skippedCount":"0","category":"User Temp"}' },
  ];
}

function customItemState(input: { settingsState: Settings }) {
  const settingsState = input.settingsState;
  const item: ScanItemInfo = { id: 'custom-path-1-item-1', targetId: 'custom-path-1', path: 'C:\\Users\\Alice\\CustomCleanup\\old.bin', size: '8192', mtimeUnixSecs: '1700000000', ageEligible: true, risk: 'Unknown', targetName: 'Custom Path (Unknown)', description: 'Custom Path selection requires typed confirmation. Unknown items require allow_delete_unknown.' };
  const statusReason = settingsState.allowDeleteUnknown ? 'Custom Path / Unknown item requires typed confirmation.' : 'Unknown selection is gated by "Allow deleting unknown folders".';
  const target: TargetScanResultDto = { targetId: 'custom-path-1', targetName: 'Custom Path (Unknown)', tier: 'Tier2', risk: 'Unknown', resolvedPaths: ['C:\\Users\\Alice\\CustomCleanup'], scanned: true, statusReason, totalBytes: item.size, eligibleBytes: item.size, fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [toScanItem(item)] };
  return { item, target };
}

function tierDefs() {
  return [
    { tier: 'Tier1' as TierDto, build: () => buildTarget({ targetId: 't-user-temp', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', description: 'Temporary files regenerated automatically.', rawItems: SAMPLE_ITEMS_T1.slice(0, 2) }) },
    { tier: 'Tier1' as TierDto, build: () => buildTarget({ targetId: 't-crash', targetName: 'Crash Dumps & WER', tier: 'Tier1', risk: 'Safe', description: 'Local crash reports and error dumps.', rawItems: SAMPLE_ITEMS_T1.slice(2) }) },
    { tier: 'Tier2' as TierDto, build: () => buildTarget({ targetId: 't-npm', targetName: 'npm Package Cache', tier: 'Tier2', risk: 'Safe', description: 'Package manager download cache, redownloaded as needed.', rawItems: SAMPLE_ITEMS_T2 }) },
  ];
}

export function makeScanResults(input: { settingsState: Settings; tiers: TierDto[] | null }) {
  const enabledList = [input.settingsState.enableTier1 ? 'Tier1' : null, input.settingsState.enableTier2 ? 'Tier2' : null, input.settingsState.enableTier3 ? 'Tier3' : null].filter(Boolean) as TierDto[];
  const enabled = new Set(input.tiers ?? enabledList);
  const itemsById = new Map<string, ScanItemInfo>();
  const targets: TargetScanResultDto[] = [];
  for (const def of tierDefs()) {
    if (!enabled.has(def.tier)) continue;
    const { target, items } = def.build();
    targets.push(target);
    for (const it of items) itemsById.set(it.id, it);
  }
  const custom = customItemState({ settingsState: input.settingsState });
  targets.push(custom.target);
  itemsById.set(custom.item.id, custom.item);
  return { targets, itemsById };
}