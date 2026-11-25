import { useEffect, useRef, useState } from 'react';
import { ReconnectingSocket } from '@/api/ws';
import type { SocketStatus } from '@/api/ws';

/**
 * Subscribe to a daemon event socket. The latest callback is kept in a ref so
 * callers can inline closures without reconnecting on every render.
 */
export function useLiveEvents(
  onEvent: (event: unknown) => void,
  path = '/ws/events',
): SocketStatus {
  const callbackRef = useRef(onEvent);
  const [status, setStatus] = useState<SocketStatus>('connecting');

  useEffect(() => {
    callbackRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    const socket = new ReconnectingSocket<unknown>(path, {
      onStatus: setStatus,
      onMessage: (event) => callbackRef.current(event),
    });
    socket.connect();
    return () => socket.close();
  }, [path]);

  return status;
}
