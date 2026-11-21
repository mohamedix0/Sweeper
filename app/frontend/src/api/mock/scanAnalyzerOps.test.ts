import type { ScanProgressEvent } from '../client';
import { createMockController } from './controller';

describe('mock scan session identity', () => {
  it('stops emitting progress when reset removes the delayed scan session', async () => {
    const client = createMockController();
    client.setScanStepDelayMs(40);
    const events: ScanProgressEvent[] = [];
    const scan = client.startScan(null, (event) => events.push(event));

    client.reset();

    await expect(scan).resolves.toEqual({ sessionId: 'mock-session-1' });
    expect(events.map((event) => event.kind)).toEqual(['started']);
  });

  it('never reads a replacement session that reuses the same id', async () => {
    const client = createMockController();
    client.setScanStepDelayMs(40);
    const firstEvents: ScanProgressEvent[] = [];
    const secondEvents: ScanProgressEvent[] = [];
    const first = client.startScan(null, (event) => firstEvents.push(event));

    client.reset();
    const second = client.startScan(null, (event) => secondEvents.push(event));

    await expect(Promise.all([first, second])).resolves.toEqual([
      { sessionId: 'mock-session-1' },
      { sessionId: 'mock-session-1' },
    ]);
    expect(firstEvents.map((event) => event.kind)).toEqual(['started']);
    expect(secondEvents.map((event) => event.kind)).toContain('completed');
  });
});
