import { describe, expect, it } from 'vitest';
import { backoffDelay, wsUrl } from '../api/ws';

describe('backoffDelay', () => {
  it('grows exponentially from 1s and never exceeds 15s', () => {
    const fixed = (): number => 0.5;
    let previous = 0;
    for (let attempt = 0; attempt < 4; attempt += 1) {
      const delay = backoffDelay(attempt, 1_000, 15_000, fixed);
      expect(delay).toBeGreaterThan(previous);
      expect(delay).toBeLessThanOrEqual(15_000);
      previous = delay;
    }
    expect(backoffDelay(20, 1_000, 15_000, fixed)).toBe(15_000);
    expect(backoffDelay(12, 1_000, 15_000, fixed)).toBeLessThanOrEqual(15_000);
  });

  it('applies bounded jitter', () => {
    const low = backoffDelay(0, 1_000, 15_000, () => 0);
    const high = backoffDelay(0, 1_000, 15_000, () => 1);
    expect(low).toBe(850);
    expect(high).toBe(1_150);
  });
});

describe('wsUrl', () => {
  it('builds same-origin API WebSocket URLs in a bare Node environment', () => {
    expect(wsUrl('/kiosk/events')).toBe('/api/v1/kiosk/events');
    expect(wsUrl('instances/demo/exec')).toBe('/api/v1/instances/demo/exec');
    expect(wsUrl('/ws/setup', { token: 'abc' })).toBe('/api/v1/ws/setup?token=abc');
  });
});
