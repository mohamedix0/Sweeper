import { describe, expect, it } from 'vitest';
import { createMockController } from './controller';
import { evaluateCustomPath, LINK_POINT_SEGMENT, MISSING_SEGMENT, protectionReason } from './pathPolicy';

const verdict = (path: string) => evaluateCustomPath(path);

describe('mock Custom Path validation mirrors Policy', () => {
  it('accepts a valid, deep Custom Path and normalizes separators', () => {
    expect(verdict('C:/Temp/Cache/Logs')).toEqual({
      path: 'C:\\Temp\\Cache\\Logs',
      allowed: true,
      reason: 'ok',
    });
  });

  it('accepts a benign OneDrive look-alike (sync-root name boundary)', () => {
    expect(verdict('C:\\Temp\\OneDriveCache\\Logs').allowed).toBe(true);
  });

  it('rejects a real OneDrive sync root anywhere in the path', () => {
    const result = verdict('C:\\Temp\\OneDrive - Contoso\\Cache');
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('Subtree-Protected Path');
  });

  it('rejects drive roots and paths shallower than 3 segments', () => {
    for (const path of ['C:\\', 'C:\\Temp', 'C:\\Temp\\Cache']) {
      expect(verdict(path).reason).toContain('depth >= 3');
    }
  });

  it('rejects descendants of every Protected Container Root', () => {
    for (const path of [
      'C:\\Windows\\Vendor\\Logs',
      'C:\\Users\\Alice\\CustomCleanup',
      'C:\\ProgramData\\Vendor\\Logs',
    ]) {
      const result = verdict(path);
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('protected container root');
    }
    expect(protectionReason(['Temp', 'Cache', 'Logs'])).toBeNull();
  });

  it('rejects descendants of a Protected Container Root', () => {
    const result = verdict('C:\\Users\\Alice\\CustomCleanup');
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('protected container root');
  });

  it('rejects Subtree-Protected Paths by fixed prefix and by segment name', () => {
    for (const path of ['C:\\Windows\\System32\\drivers', 'C:\\Temp\\Cache\\.git\\objects', 'C:\\Temp\\Cache\\.ssh\\keys']) {
      const result = verdict(path);
      expect(result.allowed).toBe(false);
      expect(result.reason).toContain('Subtree-Protected Path');
    }
  });

  it('never follows a Link Point in the Custom Path chain', () => {
    const linkPath = `C:\\Temp\\Cache\\${LINK_POINT_SEGMENT}`;
    expect(verdict(linkPath).reason).toContain('Link Point');
    expect(verdict(`C:\\Temp\\${LINK_POINT_SEGMENT}\\Cache\\Logs`).reason).toContain('Link Point');
  });

  it('rejects missing paths without falling back to a parent', () => {
    const result = verdict(`C:\\Temp\\${MISSING_SEGMENT}\\Cache`);
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('does not exist (no fallback to parent)');
  });

  it('rejects traversal and non-drive paths', () => {
    expect(verdict('relative\\path\\here').reason).toContain('absolute drive path');
    expect(verdict('C:\\Temp\\..\\Cache').reason).toContain('absolute drive path');
  });
});

describe('mock validateCustomPath command', () => {
  it('throws a typed InvalidInput error on empty input', async () => {
    const controller = createMockController();
    await expect(controller.validateCustomPath('   ')).rejects.toMatchObject({
      code: 'invalidInput',
      details: { reason: 'path is required' },
    });
  });

  it('returns a denied verdict for a rejected path', async () => {
    const controller = createMockController();
    const result = await controller.validateCustomPath('C:\\Users\\Alice\\CustomCleanup');
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('protected container root');
  });

  it('returns an allowed verdict for a valid Custom Path', async () => {
    const controller = createMockController();
    const result = await controller.validateCustomPath('C:\\Temp\\CustomCleanup\\Cache');
    expect(result).toEqual({ path: 'C:\\Temp\\CustomCleanup\\Cache', allowed: true, reason: 'ok' });
  });
});
