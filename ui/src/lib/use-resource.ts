import { useCallback, useEffect, useRef, useState } from 'react';
import type { ApiError } from '@/api/client';
import { isApiError } from '@/api/client';
import { useInterval } from './useInterval';

export interface ResourceState<T> {
  data: T | null;
  error: ApiError | null;
  loading: boolean;
  /** True while a background refresh (polling) is in flight. */
  refreshing: boolean;
  reload: () => void;
  /** Feed a fresh payload received out-of-band (e.g. a WebSocket event). */
  update: (next: T) => void;
}

interface ResourceOptions {
  /** Poll cadence in milliseconds; null disables polling. */
  pollMs?: number | null;
}

/**
 * Small async resource hook: initial load, manual reload and optional polling.
 * A failed poll keeps the last good data visible and surfaces the error.
 */
export function useResource<T>(
  loader: () => Promise<T>,
  deps: readonly unknown[],
  options: ResourceOptions = {},
): ResourceState<T> {
  const pollMs = options.pollMs ?? null;
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const loaderRef = useRef(loader);
  const requestRef = useRef(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    loaderRef.current = loader;
  }, [loader]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const run = useCallback((background: boolean) => {
    const requestId = requestRef.current + 1;
    requestRef.current = requestId;
    if (background) setRefreshing(true);
    else setLoading(true);
    void loaderRef
      .current()
      .then((result) => {
        if (!mountedRef.current || requestRef.current !== requestId) return;
        setData(result);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!mountedRef.current || requestRef.current !== requestId) return;
        if (isApiError(cause)) {
          setError(cause);
        } else {
          setError(null);
        }
      })
      .finally(() => {
        if (!mountedRef.current || requestRef.current !== requestId) return;
        setLoading(false);
        setRefreshing(false);
      });
  }, []);

  useEffect(() => {
    run(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const reload = useCallback(() => run(true), [run]);

  const update = useCallback((next: T) => {
    setData(next);
    setError(null);
  }, []);

  useInterval(() => run(true), pollMs);

  return { data, error, loading, refreshing, reload, update };
}
