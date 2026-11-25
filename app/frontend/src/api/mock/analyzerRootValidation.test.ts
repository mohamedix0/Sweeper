import type { AnalyzerProgressEvent } from '../client';
import { createMockController } from './controller';

/**
 * Regression coverage for issue #144: the mock `startAnalyzer` must apply the
 * same lexical root validation as backend `validate_analyzer_root`. These are
 * synthetic paths only (no real user data).
 */
describe('mock startAnalyzer root validation', () => {
  const invalidRoots: ReadonlyArray<readonly [string, string]> = [
    ['relative', 'relative\\path\\here'],
    ['UNC', '\\\\server\\share'],
    ['traversal (..)', 'C:\\Temp\\..\\Cache'],
    ['device prefix', '\\\\?\\C:\\Temp\\Cache'],
    ['empty segment', 'C:\\Temp\\\\Cache'],
    ['alternate data stream', 'C:\\Temp\\Cache:stream'],
  ];

  for (const [label, root] of invalidRoots) {
    it(`rejects a ${label} root with a typed invalidInput error`, async () => {
      const client = createMockController();
      const events: AnalyzerProgressEvent[] = [];

      await expect(client.startAnalyzer(root, (e) => events.push(e))).rejects.toMatchObject({
        code: 'invalidInput',
        details: { reason: expect.stringContaining('absolute drive path') },
      });
      // A rejected root must not emit progress or consume a session id.
      expect(events).toEqual([]);
      await expect(client.startAnalyzer('C:\\')).resolves.toMatchObject({ sessionId: 'mock-analyzer-1' });
    });
  }

  it('rejects a missing analyzer root before emitting progress', async () => {
    const client = createMockController();
    const events: AnalyzerProgressEvent[] = [];
    await expect(client.startAnalyzer('C:\\Temp\\missing', (e) => events.push(e))).rejects.toMatchObject({
      code: 'invalidInput',
      details: { reason: expect.stringContaining('does not exist') },
    });
    expect(events).toEqual([]);
  });

  it('rejects a link-point analyzer root before emitting progress', async () => {
    const client = createMockController();
    const events: AnalyzerProgressEvent[] = [];
    await expect(client.startAnalyzer('C:\\Temp\\linkpoint', (e) => events.push(e))).rejects.toMatchObject({
      code: 'invalidInput',
      details: { reason: expect.stringContaining('Link Point') },
    });
    expect(events).toEqual([]);
  });

  it('defaults null, empty and whitespace roots to C:\\', async () => {
    for (const root of [null, '', '   ']) {
      const client = createMockController();
      const result = await client.startAnalyzer(root);
      expect(result.rootPath).toBe('C:\\');
    }
  });

  it('accepts an explicit protected root for display/analysis', async () => {
    const client = createMockController();
    const result = await client.startAnalyzer('C:\\Users\\Alice');
    expect(result.rootPath).toBe('C:\\Users\\Alice');
  });

  it('normalizes separators and drive casing for a valid root', async () => {
    const client = createMockController();
    const result = await client.startAnalyzer('c:/Temp/Cache');
    expect(result.rootPath).toBe('C:\\Temp\\Cache');
  });
});
