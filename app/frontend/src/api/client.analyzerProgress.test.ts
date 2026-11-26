import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AnalyzerProgressEvent, StartAnalyzerResult } from './client';

type ProgressListener = (event: { payload: AnalyzerProgressEvent }) => void;

const h = vi.hoisted(() => {
  let emit: ((payload: AnalyzerProgressEvent) => void) | null = null;
  let finish: ((result: StartAnalyzerResult) => void) | null = null;
  const unlisten = vi.fn();
  return {
    unlisten,
    listen: vi.fn(async (cb: ProgressListener) => {
      emit = (payload) => cb({ payload });
      return unlisten;
    }),
    invoke: vi.fn((_args: { root: string | null }) => new Promise<{ status: 'ok'; data: StartAnalyzerResult }>((resolve) => {
      finish = (result) => resolve({ status: 'ok', data: result });
    })),
    emit: (payload: AnalyzerProgressEvent) => emit?.(payload),
    finish: (result: StartAnalyzerResult) => finish?.(result),
  };
});

vi.mock('../contract/bindings', () => ({
  commands: { startAnalyzer: (args: { root: string | null }) => h.invoke(args) },
  events: { analyzerProgressEvent: { listen: h.listen } },
}));

import { createTauriClient } from './client';

function progress(sessionId: string, scannedDirs: string, isStart = false): AnalyzerProgressEvent {
  return { sessionId, isStart, scannedDirs, scannedFiles: '0', scannedBytes: '0', currentPath: null };
}

function result(sessionId: string): StartAnalyzerResult {
  return {
    sessionId,
    rootPath: 'C:\\',
    totalBytes: '0',
    totalItems: '0',
    scannedDirs: '0',
    scannedFiles: '0',
    cancelled: false,
    warnings: [],
    root: {} as StartAnalyzerResult['root'],
  };
}

describe('startAnalyzer progress session isolation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('latches on the start marker and filters another session before and after resolution', async () => {
    const received: AnalyzerProgressEvent[] = [];
    const pending = createTauriClient().startAnalyzer('C:\\', (e) => received.push(e));
    await vi.waitFor(() => expect(h.listen).toHaveBeenCalledTimes(1));

    h.emit(progress('an-B', '1', true));
    expect(received.map((e) => e.sessionId)).toEqual(['an-B']);

    // A cancelled run's stray event is held before the marker and dropped.
    h.emit(progress('an-A', '9'));
    h.finish(result('an-B'));
    await pending;
    h.emit(progress('an-A', '8'));
    h.emit(progress('an-B', '2'));

    expect(received.map((e) => e.sessionId)).toEqual(['an-B', 'an-B']);
    expect(h.unlisten).toHaveBeenCalledTimes(1);
  });

  it('ignores a stale pre-start event and replays only the new run\'s held events', async () => {
    const received: AnalyzerProgressEvent[] = [];
    const pending = createTauriClient().startAnalyzer('C:\\', (e) => received.push(e));
    await vi.waitFor(() => expect(h.listen).toHaveBeenCalledTimes(1));

    // A cancelled run's late event arrives first and is never latched.
    h.emit(progress('an-A', '9'));
    h.emit(progress('an-B', '1'));
    h.emit(progress('an-B', '2'));
    expect(received).toEqual([]);

    // The authoritative start marker binds the run and releases its held events.
    h.emit(progress('an-B', '0', true));
    h.finish(result('an-B'));
    await pending;

    expect(received.map((e) => e.sessionId)).toEqual(['an-B', 'an-B', 'an-B']);
    expect(received.map((e) => e.scannedDirs)).toEqual(['0', '1', '2']);
    expect(h.unlisten).toHaveBeenCalledTimes(1);
  });
});
