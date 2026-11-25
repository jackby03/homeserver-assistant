/**
 * Typed JSON client for the `hs-daemon` HTTP API.
 *
 * Responsibilities:
 * - base URL resolution (`VITE_API_BASE`, otherwise same-origin dev proxy)
 * - credentialed requests (`credentials: 'include'`)
 * - CSRF header on mutating requests, except login
 * - uniform {@link ApiError} for the `{ error: { code, message, details } }` shape
 * - an `hs:unauthorized` DOM event when the session expires
 */

export const API_PREFIX = '/api/v1';

const rawBase = (import.meta.env.VITE_API_BASE ?? '').trim();
/** Configured base without trailing slash. Empty string means same origin. */
export const API_BASE = rawBase.replace(/\/+$/, '');
/** Fully qualified API root, e.g. `/api/v1` or `https://host/api/v1`. */
export const API_ROOT = `${API_BASE}${API_PREFIX}`;

/** Event dispatched when the daemon answers 401. */
export const UNAUTHORIZED_EVENT = 'hs:unauthorized';

const CSRF_COOKIE = 'hs_csrf';
const MUTATING_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly details: unknown;

  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export function isApiError(value: unknown): value is ApiError {
  return value instanceof ApiError;
}

/** True for errors caused by an unreachable daemon (offline / proxy down). */
export function isOfflineError(error: unknown): boolean {
  return isApiError(error) && error.status === 0;
}

/** Parse the daemon error envelope from an arbitrary decoded JSON body. */
export function parseApiErrorBody(body: unknown): {
  code: string;
  message: string;
  details?: unknown;
} | null {
  if (typeof body !== 'object' || body === null) return null;
  const envelope = (body as { error?: unknown }).error;
  if (typeof envelope !== 'object' || envelope === null) return null;
  const { code, message, details } = envelope as {
    code?: unknown;
    message?: unknown;
    details?: unknown;
  };
  if (typeof code !== 'string' || typeof message !== 'string') return null;
  return details === undefined ? { code, message } : { code, message, details };
}

/** Map an HTTP status to a stable fallback error code. */
export function fallbackErrorCode(status: number): string {
  switch (status) {
    case 400:
      return 'bad_request';
    case 401:
      return 'unauthorized';
    case 403:
      return 'forbidden';
    case 404:
      return 'not_found';
    case 409:
      return 'conflict';
    case 422:
      return 'validation_error';
    case 429:
      return 'rate_limited';
    default:
      return status >= 500 ? 'internal_error' : 'request_failed';
  }
}

/** Read a cookie by name, tolerating unavailable `document` (SSR/tests). */
export function readCookie(name: string): string | null {
  if (typeof document === 'undefined') return null;
  const prefix = `${name}=`;
  const parts = document.cookie.split(';');
  for (const part of parts) {
    const candidate = part.trim();
    if (candidate.startsWith(prefix)) {
      return decodeURIComponent(candidate.slice(prefix.length));
    }
  }
  return null;
}

let csrfOverride: string | null = null;

/** Cache the CSRF token returned by login/me so it is available immediately. */
export function setCsrfToken(token: string | null): void {
  csrfOverride = token !== null && token !== '' ? token : null;
}

export function getCsrfToken(): string | null {
  return csrfOverride ?? readCookie(CSRF_COOKIE);
}

/**
 * Resolve the CSRF header value for a request.
 *
 * Login is exempt because the browser cannot know the token before the first
 * authenticated response. Everything else mutating must carry it.
 */
export function csrfHeaderFor(method: string, path: string): string | null {
  const upper = method.toUpperCase();
  if (!MUTATING_METHODS.has(upper)) return null;
  if (path === '/auth/login') return null;
  return getCsrfToken();
}

export interface RequestOptions {
  /** JSON body; serialized automatically. */
  body?: unknown;
  /** Raw body for endpoints that do not accept JSON. */
  rawBody?: BodyInit;
  headers?: Record<string, string>;
  signal?: AbortSignal;
  /** Skip the global 401 event (used by silent session probes). */
  skipUnauthorizedEvent?: boolean;
}

async function buildHeaders(
  method: string,
  path: string,
  options: RequestOptions,
): Promise<Headers> {
  const headers = new Headers(options.headers);
  headers.set('Accept', 'application/json');
  const csrf = csrfHeaderFor(method, path);
  if (csrf !== null) headers.set('X-HS-CSRF', csrf);
  if (options.body !== undefined && options.rawBody === undefined) {
    headers.set('Content-Type', 'application/json');
  }
  return headers;
}

async function errorFromResponse(response: Response): Promise<ApiError> {
  let payload: unknown = null;
  try {
    payload = await response.json();
  } catch {
    /* the daemon always answers JSON; tolerate empty bodies from proxies */
  }
  const parsed = parseApiErrorBody(payload);
  if (parsed) {
    return new ApiError(response.status, parsed.code, parsed.message, parsed.details);
  }
  const message = response.statusText.trim() !== '' ? response.statusText : 'Request failed';
  return new ApiError(response.status, fallbackErrorCode(response.status), message);
}

/** Core request function; prefer the typed helpers below. */
export async function request<T>(
  method: string,
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const url = `${API_ROOT}${path}`;
  const upper = method.toUpperCase();
  const init: RequestInit = {
    method: upper,
    credentials: 'include',
    headers: await buildHeaders(upper, path, options),
  };
  if (options.body !== undefined) init.body = JSON.stringify(options.body);
  else if (options.rawBody !== undefined) init.body = options.rawBody;
  if (options.signal !== undefined) init.signal = options.signal;

  let response: Response;
  try {
    response = await fetch(url, init);
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') throw cause;
    throw new ApiError(0, 'network_error', 'Unable to reach the appliance.');
  }

  if (!response.ok) {
    const error = await errorFromResponse(response);
    if (error.status === 401 && !options.skipUnauthorizedEvent) {
      setCsrfToken(null);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(new CustomEvent(UNAUTHORIZED_EVENT));
      }
    }
    throw error;
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (text.trim() === '') return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new ApiError(response.status, 'invalid_response', 'The appliance returned invalid JSON.');
  }
}

export const http = {
  get: <T>(path: string, options?: Omit<RequestOptions, 'body'>): Promise<T> =>
    request<T>('GET', path, options ?? {}),
  post: <T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> =>
    request<T>('POST', path, { ...options, body }),
  put: <T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> =>
    request<T>('PUT', path, { ...options, body }),
  patch: <T>(path: string, body?: unknown, options?: RequestOptions): Promise<T> =>
    request<T>('PATCH', path, { ...options, body }),
  delete: <T>(path: string, options?: RequestOptions): Promise<T> =>
    request<T>('DELETE', path, options ?? {}),
};
