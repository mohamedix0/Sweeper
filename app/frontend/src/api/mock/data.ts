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
  let total = 0n; let eligible = 0n; let eligibleCount = 0n; let skipped = 0n;
  for (const it of items) {
    const sz = BigInt(it.size); total += sz;
    if (it.ageEligible) { eligible += sz; eligibleCount += 1n; } else { skipped += 1n; }
  }
  return { total, eligible, eligibleCount, skipped };
}

function toScanItem(item: ScanItemInfo): ScanItemDto {
  const { risk: _r, targetName: _n, description: _d, ...rest } = item;
  return rest;
}

export function buildTarget(args: { targetId: string; targetName: string; tier: TierDto; risk: RiskDto; description: string; rawItems: Array<Omit<ScanItemDto, 'id' | 'targetId'> & { targetId?: string }> }) {
  const items = args.rawItems.map((it, i) => ({ ...it, id: `${args.targetId}-item-${i + 1}`, risk: args.risk, targetName: args.targetName, description: args.description })) as ScanItemInfo[];
  const s = itemStats(items);
  const target: TargetScanResultDto = {
    targetId: args.targetId, targetName: args.targetName, tier: args.tier, risk: args.risk,
    resolvedPaths: items.length ? [items[0].path.replace(/\\[^\\]+$/, '')] : [],
    scanned: true, statusReason: null,
    totalBytes: s.total.toString(), eligibleBytes: s.eligible.toString(),
    fileCount: BigInt(items.length).toString(), eligibleCount: s.eligibleCount.toString(), skippedCount: s.skipped.toString(),
    items: items.map(toScanItem),
  };
  return { target, items };
}

export function makeNode(partial: Omit<AnalyzerNodeDto, 'children'> & { children?: AnalyzerNodeDto[] }): AnalyzerNodeDto {
  return { ...partial, children: partial.children ?? [] };
}

function node(id: string, name: string, path: string, opts: Partial<AnalyzerNodeDto> & { children?: AnalyzerNodeDto[] }): AnalyzerNodeDto {
  return makeNode({ id, name, path, isDir: false, sizeBytes: '0', itemCount: '1', risk: 'Safe', description: '', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, ...opts });
}

export function buildDeepBranch(base: string): AnalyzerNodeDto {
  const root = `${base}\\Users\\Alice\\AppData\\Local\\Packages\\DeepVendor`;
  let leaf: AnalyzerNodeDto = node('an-deep-leaf', 'cache.bin', `${root}\\layer30\\cache.bin`, { sizeBytes: '2048', risk: 'Safe', description: 'Deep nested cache file (Safe).', tier: 'Tier2', targetId: 'mock-deep-safe', children: [] });
  for (let i = 30; i >= 1; i--) {
    leaf = node(`an-deep-dir-${i}`, `layer${i}`, `${root}\\layer${i}`, { isDir: true, sizeBytes: String(2048 + i), itemCount: String(31 - i), description: 'Deep nested directory for virtualization testing.', tier: 'Tier2', targetId: 'mock-deep-safe', children: [leaf] });
  }
  return node('an-deep-root', 'DeepVendor', root, { isDir: true, sizeBytes: '4096', itemCount: '31', description: 'Deep nested structure (virtualization demo).', tier: 'Tier2', targetId: 'mock-deep-safe', children: [leaf] });
}

export function buildBulkFiles(base: string): AnalyzerNodeDto[] {
  const files: AnalyzerNodeDto[] = [];
  for (let i = 1; i <= 40; i++) {
    files.push(node(`an-bulk-file-${i}`, `temp-${i}.tmp`, `${base}\\Users\\Alice\\AppData\\Local\\Temp\\bulk\\temp-${i}.tmp`, { sizeBytes: String(1024 * i), description: 'Temporary file (Safe).', tier: 'Tier1', targetId: 'mock-temp' }));
  }
  return files;
}

function sumSizes(nodes: AnalyzerNodeDto[]): bigint { return nodes.reduce((a, n) => a + BigInt(n.sizeBytes), 0n); }
function sumItems(nodes: AnalyzerNodeDto[]): bigint { return nodes.reduce((a, n) => a + BigInt(n.itemCount), 0n); }

function protectedGit(base: string) {
  const root = `${base}\\Dev\\repo\\.git`;
  const child = node('an-protected-git-obj', 'objects', `${root}\\objects`, { isDir: true, sizeBytes: '2048', itemCount: '4', risk: 'Protected', description: 'Protected git internals.', reason: 'Protected Path', displayOnly: true, selectable: false });
  return node('an-protected-git', '.git', root, { isDir: true, sizeBytes: '4096', itemCount: '12', risk: 'Protected', description: 'Protected repository metadata (.git).', reason: 'Protected Path', displayOnly: true, selectable: false, children: [child] });
}

function specialNodes(base: string) {
  const linkPoint = node('an-link-point', 'OneDriveLink', `${base}\\Users\\Alice\\OneDriveLink`, { isDir: true, itemCount: '0', risk: 'Caution', description: 'Link Point (reparse point). Never followed.', reason: 'Link Point — not traversed', isLinkPoint: true, displayOnly: true, selectable: false });
  const vhdx = node('an-vhdx', 'dev-machine.vhdx', `${base}\\Users\\Alice\\Virtual Machines\\dev-machine.vhdx`, { sizeBytes: '1073741824', risk: 'Caution', description: 'Virtual disk image (.vhdx). Display-only by policy.', reason: '.vhdx display-only', displayOnly: true, selectable: false });
  const cloud = node('an-cloud', 'CloudPlaceholder.txt', `${base}\\Users\\Alice\\OneDrive\\CloudPlaceholder.txt`, { itemCount: '1', risk: 'Caution', description: 'Cloud placeholder file. Skipped by policy.', reason: 'Cloud placeholder — skipped', isCloudPlaceholder: true, displayOnly: true, selectable: false });
  return [linkPoint, vhdx, cloud];
}

function unknownFolder(base: string, allowUnknown: boolean) {
  const reason = allowUnknown ? 'Unknown (selectable when explicitly allowed)' : 'Unknown items are not selectable until allowed in Settings';
  const file = node('an-unknown-file', 'old.bin', `${base}\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache\\old.bin`, { sizeBytes: '8192', risk: 'Unknown', description: 'Unknown cached data file.', reason, selectable: allowUnknown });
  return node('an-unknown-folder', 'MysteryCache', `${base}\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache`, { isDir: true, sizeBytes: '8192', itemCount: '2', risk: 'Unknown', description: 'Unknown folder not in Knowledge Base. May require confirmation/policy to remove.', reason, selectable: allowUnknown, children: [file] });
}

function cautionFolder(base: string) {
  const log1 = node('an-caution-log1', 'app.log', `${base}\\ProgramData\\Vendor\\Logs\\app.log`, { sizeBytes: '131072', risk: 'Caution', description: 'Application log file.', tier: 'Tier2', targetId: 'mock-caution-logs' });
  const log2 = node('an-caution-log2', 'service.log', `${base}\\ProgramData\\Vendor\\Logs\\service.log`, { sizeBytes: '131072', risk: 'Caution', description: 'Service log file.', tier: 'Tier2', targetId: 'mock-caution-logs' });
  return node('an-caution-logs', 'Logs', `${base}\\ProgramData\\Vendor\\Logs`, { isDir: true, sizeBytes: '262144', itemCount: '8', risk: 'Caution', description: 'Vendor logs (Caution). Review before removal.', tier: 'Tier2', targetId: 'mock-caution-logs', children: [log1, log2] });
}

function safeTemp(base: string) {
  const manyFiles = buildBulkFiles(base);
  const bulk = node('an-safe-temp-bulk', 'bulk', `${base}\\Users\\Alice\\AppData\\Local\\Temp\\bulk`, { isDir: true, sizeBytes: sumSizes(manyFiles).toString(), itemCount: String(manyFiles.length), description: 'Bulk temporary files (Safe).', tier: 'Tier1', targetId: 'mock-temp', children: manyFiles });
  const a = node('an-safe-temp-a', 'a.tmp', `${base}\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp`, { sizeBytes: '1048576', description: 'Temporary file.', tier: 'Tier1', targetId: 'mock-temp' });
  const recent = node('an-safe-temp-recent', 'recent.log', `${base}\\Users\\Alice\\AppData\\Local\\Temp\\recent.log`, { sizeBytes: '2048', description: 'Recent temp log (not age-eligible in scan, but analyzer node shown for selection policy demo).', reason: 'Recent (demo)', tier: 'Tier1', targetId: 'mock-temp' });
  const kids = [a, recent, bulk];
  return node('an-safe-temp', 'Temp', `${base}\\Users\\Alice\\AppData\\Local\\Temp`, { isDir: true, sizeBytes: sumSizes(kids).toString(), itemCount: String(2 + manyFiles.length), description: 'User temporary files (Safe).', tier: 'Tier1', targetId: 'mock-temp', children: kids });
}

export function buildAnalyzerTree(rootPath: string, allowUnknown: boolean): AnalyzerNodeDto {
  const base = rootPath.replace(/[\\/]+$/, '');
  const children = [safeTemp(base), cautionFolder(base), unknownFolder(base, allowUnknown), protectedGit(base), ...specialNodes(base), buildDeepBranch(base)];
  return node('an-root', base || rootPath, rootPath, { isDir: true, sizeBytes: sumSizes(children).toString(), itemCount: sumItems(children).toString(), risk: 'Caution', description: `Analyzer root for ${rootPath}. Expand to review folders and select eligible items for Review & Clean.`, reason: 'Drive root — expand to select specific folders', displayOnly: false, selectable: false, children });
}

export function indexAnalyzerNodes(root: AnalyzerNodeDto): Map<string, AnalyzerNodeDto> {
  const map = new Map<string, AnalyzerNodeDto>();
  const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
  walk(root); return map;
}

export function defaultSeedLogs(nowMs = Date.now()) {
  return [
    { id: 'log-1', name: '2026-11-13-scan.jsonl', modifiedUnixSecs: String(Math.floor(nowMs / 1000) - 3 * 24 * 60 * 60), sizeBytes: '3072', content: '{"timestamp":"2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp","size":"1048576","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"Crash Dumps & WER","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps","size":"5242880","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"npm Package Cache","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache","size":"7340032","status":"Eligible","kind":"Category"}\n{"timestamp":"2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\recent.log","size":"2048","status":"Skipped · < 24h","kind":"File"}\n{"timestamp":"2:00 AM","category":"Custom Path (Unknown)","path":"C:\\\\Users\\\\Alice\\\\CustomCleanup\\\\old.bin","size":"8192","status":"Skipped · Unverified","kind":"File"}\n{"record":"outcome","status":"completed","event":"scan","categoriesCount":4,"items":5,"totalBytes":"13639680","timestamp":"2:00 AM"}' },
    { id: 'log-2', name: '2026-11-12-clean.jsonl', modifiedUnixSecs: String(Math.floor(nowMs / 1000) - 4 * 24 * 60 * 60), sizeBytes: '1024', content: '{"timestamp":"Yesterday, 2:00 AM","category":"User Temp","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\Temp\\\\a.tmp","size":"1048576","status":"Cleaned"}\n{"timestamp":"Yesterday, 2:00 AM","category":"Crash Dumps & WER","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\CrashDumps\\\\app.dmp","size":"5242880","status":"Cleaned"}\n{"timestamp":"Yesterday, 2:00 AM","category":"npm Package Cache","path":"C:\\\\Users\\\\Alice\\\\AppData\\\\Local\\\\npm-cache\\\\index-v5","size":"7340032","status":"Cleaned"}\n{"record":"outcome","status":"completed","event":"clean","deletedCount":"3","bytesFreed":"13631488","skippedCount":"0","category":"User Temp"}' },
  ];
}

function customItemState(settingsState: Settings) {
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

export function makeScanResults(settingsState: Settings, tiers: TierDto[] | null) {
  const enabled = new Set(tiers ?? ([settingsState.enableTier1 ? 'Tier1' : null, settingsState.enableTier2 ? 'Tier2' : null, settingsState.enableTier3 ? 'Tier3' : null].filter(Boolean) as TierDto[]));
  const itemsById = new Map<string, ScanItemInfo>();
  const targets: TargetScanResultDto[] = [];
  for (const def of tierDefs()) {
    if (!enabled.has(def.tier)) continue;
    const { target, items } = def.build();
    targets.push(target);
    for (const it of items) itemsById.set(it.id, it);
  }
  const custom = customItemState(settingsState);
  targets.push(custom.target);
  itemsById.set(custom.item.id, custom.item);
  return { targets, itemsById };
}
