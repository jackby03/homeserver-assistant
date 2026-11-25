/**
 * Reconnecting WebSocket helper.
 *
 * - Exponential backoff from 1s to 15s with jitter.
 * - Heartbeat ping every 20s; a silent socket for 2.5 heartbeats is considered
 *   stale and is recycled.
 * - JSON encode/decode with a caller-supplied guard.
 */

import { API_PREFIX } from './client';

export type SocketStatus = 'connecting' | 'open' | 'reconnecting' | 'closed';

export interface ReconnectingSocketOptions {
  heartbeatMs?: number;
  minDelayMs?: number;
  maxDelayMs?: number;
  /** Extra query parameters appended to the URL. */
  query?: Record<string, string>;
}

export interface ReconnectingSocketHandlers<T> {
  onMessage: (message: T) => void;
  onStatus?: (status: SocketStatus) => void;
  onOpen?: () => void;
}

function websocketOrigin(): string {
  const explicit = (import.meta.env.VITE_WS_BASE ?? '').trim();
  if (explicit !== '') return explicit.replace(/\/+$/, '');
  const apiBase = (import.meta.env.VITE_API_BASE ?? '').trim().replace(/\/+$/, '');
  if (/^https?:\/\//i.test(apiBase)) {
    return apiBase.replace(/^http/i, 'ws');
  }
  if (typeof window === 'undefined') return '';
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

/** Build a WebSocket URL for an API path such as `/kiosk/events`. */
export function wsUrl(path: string, query?: Record<string, string>): string {
  const normalized = path.startsWith('/') ? path : `/${path}`;
  const base = `${websocketOrigin()}${API_PREFIX}${normalized}`;
  if (!query || Object.keys(query).length === 0) return base;
  const params = new URLSearchParams(query);
  return `${base}?${params.toString()}`;
}

/** Compute the reconnect delay for an attempt index (0-based). */
export function backoffDelay(
  attempt: number,
  minDelayMs = 1_000,
  maxDelayMs = 15_000,
  random: () => number = Math.random,
): number {
  const exponential = Math.min(maxDelayMs, minDelayMs * 2 ** Math.max(0, attempt));
  const jitter = 0.85 + random() * 0.3;
  return Math.round(Math.min(maxDelayMs, exponential * jitter));
}

export class ReconnectingSocket<T> {
  private socket: WebSocket | null = null;
  private closedByCaller = false;
  private attempt = 0;
  private reconnectTimer: number | null = null;
  private heartbeatTimer: number | null = null;
  private lastMessageAt = 0;
  private readonly url: string;
  private readonly handlers: ReconnectingSocketHandlers<T>;
  private readonly heartbeatMs: number;
  private readonly minDelayMs: number;
  private readonly maxDelayMs: number;

  constructor(
    path: string,
    handlers: ReconnectingSocketHandlers<T>,
    options: ReconnectingSocketOptions = {},
  ) {
    this.url = wsUrl(path, options.query);
    this.handlers = handlers;
    this.heartbeatMs = options.heartbeatMs ?? 20_000;
    this.minDelayMs = options.minDelayMs ?? 1_000;
    this.maxDelayMs = options.maxDelayMs ?? 15_000;
  }

  get currentUrl(): string {
    return this.url;
  }

  connect(): void {
    if (typeof WebSocket === 'undefined') return;
    this.closedByCaller = false;
    this.open();
  }

  private emitStatus(status: SocketStatus): void {
    this.handlers.onStatus?.(status);
  }

  private open(): void {
    this.emitStatus(this.attempt === 0 ? 'connecting' : 'reconnecting');
    let socket: WebSocket;
    try {
      socket = new WebSocket(this.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.addEventListener('open', () => {
      this.attempt = 0;
      this.lastMessageAt = Date.now();
      this.emitStatus('open');
      this.handlers.onOpen?.();
      this.startHeartbeat();
    });

    socket.addEventListener('message', (event: MessageEvent<unknown>) => {
      this.lastMessageAt = Date.now();
      if (typeof event.data !== 'string') return;
      try {
        const parsed: unknown = JSON.parse(event.data);
        this.handlers.onMessage(parsed as T);
      } catch {
        /* ignore malformed frames */
      }
    });

    socket.addEventListener('close', () => {
      this.stopHeartbeat();
      this.socket = null;
      if (this.closedByCaller) {
        this.emitStatus('closed');
        return;
      }
      this.scheduleReconnect();
    });

    socket.addEventListener('error', () => {
      // `close` always follows `error`; reconnect logic lives there.
    });
  }

  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.heartbeatTimer = window.setInterval(() => {
      if (this.socket === null) return;
      if (Date.now() - this.lastMessageAt > this.heartbeatMs * 2.5) {
        this.socket.close();
        return;
      }
      this.send({ type: 'ping' });
    }, this.heartbeatMs);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer !== null) {
      window.clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.closedByCaller || this.reconnectTimer !== null) return;
    this.emitStatus('reconnecting');
    const delay = backoffDelay(this.attempt, this.minDelayMs, this.maxDelayMs);
    this.attempt += 1;
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.closedByCaller) this.open();
    }, delay);
  }

  send(message: unknown): void {
    if (this.socket === null || this.socket.readyState !== WebSocket.OPEN) return;
    this.socket.send(typeof message === 'string' ? message : JSON.stringify(message));
  }

  close(): void {
    this.closedByCaller = true;
    this.stopHeartbeat();
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.socket?.close();
    this.socket = null;
    this.emitStatus('closed');
  }
}
