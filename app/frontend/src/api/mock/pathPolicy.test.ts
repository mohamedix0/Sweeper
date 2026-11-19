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

describe('mock Custom Path validation matches backend syntax and ASCII folding', () => {
  it('folds case ASCII-only, never Unicode (no ß -> SS, no Kelvin-sign -> k)', () => {
    // Backend `to_ascii_uppercase` leaves ß unchanged, so `OneDriveß` is a sync
    // root boundary (Subtree-Protected). The mock previously allowed it because
    // `toUpperCase()` turned it into `ONEDRIVESS`, a benign look-alike.
    const sharpS = verdict('C:\\Temp\\OneDriveß\\Cache');
    expect(sharpS.allowed).toBe(false);
    expect(sharpS.reason).toContain('Subtree-Protected Path');

    // Backend `eq_ignore_ascii_case` never folds U+212A KELVIN SIGN to 'k', so
    // `.Kube` is not the protected `.kube` segment. The mock previously
    // rejected it because `toLowerCase()` folded it to `.kube`.
    const kelvin = verdict('C:\\Temp\\.\u212Aube\\Cache');
    expect(kelvin.allowed).toBe(true);
    expect(kelvin.reason).toBe('ok');
    expect(protectionReason(['Temp', '.\u212Aube', 'Cache'])).toBeNull();
  });

  it('rejects empty components before normalization', () => {
    // Backend `lexical_parse` rejects both forms as EmptySegment. The mock
    // previously repaired them with `.filter(Boolean)` / trailing-separator trim.
    for (const path of ['C:\\Temp\\\\Cache\\Logs', 'C:\\Temp\\Cache\\Logs\\']) {
      expect(verdict(path).allowed, path).toBe(false);
    }
  });

  it('rejects segment syntax the backend rejects (trailing dot and ADS)', () => {
    // Backend `validate_segment` rejects segments ending in '.', and
    // `lexical_parse` rejects ':' outside the drive position (ADS).
    for (const path of ['C:\\Temp\\.git.\\objects', 'C:\\Temp\\Cache:stream\\Logs']) {
      expect(verdict(path).allowed, path).toBe(false);
    }
  });

  it('still accepts a well-formed path mixing / and \\ separators', () => {
    expect(verdict('C:/Temp\\Cache/Logs')).toEqual({
      path: 'C:\\Temp\\Cache\\Logs',
      allowed: true,
      reason: 'ok',
    });
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
