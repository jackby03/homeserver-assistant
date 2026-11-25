/**
 * Fixed-capacity ring buffer used to feed sparklines with a bounded history.
 */
export class RingBuffer<T> {
  private readonly capacity: number;
  private readonly items: T[] = [];

  constructor(capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0) {
      throw new RangeError('RingBuffer capacity must be a positive integer');
    }
    this.capacity = capacity;
  }

  push(item: T): void {
    this.items.push(item);
    if (this.items.length > this.capacity) {
      this.items.splice(0, this.items.length - this.capacity);
    }
  }

  values(): T[] {
    return [...this.items];
  }

  get size(): number {
    return this.items.length;
  }
}
