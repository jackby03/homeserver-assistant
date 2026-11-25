import { useEffect, useRef } from 'react';

/**
 * Run a callback on an interval while `enabled` is true. The latest callback is
 * stored in a ref so callers do not need to memoize it.
 */
export function useInterval(callback: () => void, delayMs: number | null): void {
  const savedCallback = useRef(callback);

  useEffect(() => {
    savedCallback.current = callback;
  }, [callback]);

  useEffect(() => {
    if (delayMs === null) return undefined;
    const id = window.setInterval(() => savedCallback.current(), delayMs);
    return () => window.clearInterval(id);
  }, [delayMs]);
}

/** Re-render on a fixed cadence, useful for clocks. */
export function useNow(intervalMs = 1000): Date {
  const [, force] = useReducerTick();
  useInterval(() => force(), intervalMs);
  return new Date();
}

function useReducerTick(): [number, () => void] {
  const ref = useRef(0);
  const force = useRef<() => void>(() => undefined);
  const [, setCount] = useStateTuple();
  force.current = () => setCount((count) => count + 1);
  return [ref.current, () => force.current()];
}

// Local re-export to keep the hook file dependency-free at module top level.
import { useState } from 'react';

function useStateTuple(): [number, (updater: (previous: number) => number) => void] {
  return useState(0);
}
