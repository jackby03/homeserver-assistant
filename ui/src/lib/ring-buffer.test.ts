import { describe, expect, it } from 'vitest';
import { RingBuffer } from './ring-buffer';

describe('RingBuffer', () => {
  it('keeps only the most recent values', () => {
    const buffer = new RingBuffer<number>(3);
    buffer.push(1);
    buffer.push(2);
    buffer.push(3);
    buffer.push(4);
    expect(buffer.values()).toEqual([2, 3, 4]);
    expect(buffer.size).toBe(3);
  });

  it('returns a copy of the internal data', () => {
    const buffer = new RingBuffer<number>(2);
    buffer.push(1);
    const values = buffer.values();
    values.push(99);
    expect(buffer.values()).toEqual([1]);
  });

  it('rejects invalid capacities', () => {
    expect(() => new RingBuffer<number>(0)).toThrow(RangeError);
    expect(() => new RingBuffer<number>(1.5)).toThrow(RangeError);
  });
});
