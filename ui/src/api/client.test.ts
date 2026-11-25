import { beforeEach, describe, expect, it } from 'vitest';
import {
  ApiError,
  csrfHeaderFor,
  fallbackErrorCode,
  parseApiErrorBody,
  setCsrfToken,
} from './client';

describe('parseApiErrorBody', () => {
  it('parses the daemon error envelope', () => {
    expect(
      parseApiErrorBody({
        error: { code: 'validation_error', message: 'bad input', details: { field: 'name' } },
      }),
    ).toEqual({
      code: 'validation_error',
      message: 'bad input',
      details: { field: 'name' },
    });
  });

  it('omits missing details', () => {
    expect(parseApiErrorBody({ error: { code: 'not_found', message: 'gone' } })).toEqual({
      code: 'not_found',
      message: 'gone',
    });
  });

  it('rejects non-envelope payloads', () => {
    expect(parseApiErrorBody(null)).toBeNull();
    expect(parseApiErrorBody('boom')).toBeNull();
    expect(parseApiErrorBody({ error: 'boom' })).toBeNull();
    expect(parseApiErrorBody({ error: { code: 500, message: 'x' } })).toBeNull();
    expect(parseApiErrorBody({ error: { code: 'x' } })).toBeNull();
  });
});

describe('fallbackErrorCode', () => {
  it('maps common HTTP statuses', () => {
    expect(fallbackErrorCode(400)).toBe('bad_request');
    expect(fallbackErrorCode(401)).toBe('unauthorized');
    expect(fallbackErrorCode(403)).toBe('forbidden');
    expect(fallbackErrorCode(404)).toBe('not_found');
    expect(fallbackErrorCode(409)).toBe('conflict');
    expect(fallbackErrorCode(422)).toBe('validation_error');
    expect(fallbackErrorCode(429)).toBe('rate_limited');
    expect(fallbackErrorCode(500)).toBe('internal_error');
    expect(fallbackErrorCode(503)).toBe('internal_error');
    expect(fallbackErrorCode(418)).toBe('request_failed');
  });
});

describe('CSRF header policy', () => {
  beforeEach(() => {
    setCsrfToken(null);
  });

  it('is only attached to mutating requests', () => {
    setCsrfToken('token-123');
    expect(csrfHeaderFor('GET', '/instances')).toBeNull();
    expect(csrfHeaderFor('HEAD', '/instances')).toBeNull();
    expect(csrfHeaderFor('POST', '/instances')).toBe('token-123');
    expect(csrfHeaderFor('PUT', '/network/dns')).toBe('token-123');
    expect(csrfHeaderFor('PATCH', '/instances/a/limits')).toBe('token-123');
    expect(csrfHeaderFor('DELETE', '/instances/a')).toBe('token-123');
    expect(csrfHeaderFor('post', '/apps')).toBe('token-123');
  });

  it('never attaches a CSRF header to login', () => {
    setCsrfToken('token-123');
    expect(csrfHeaderFor('POST', '/auth/login')).toBeNull();
  });

  it('returns null when no token is known', () => {
    expect(csrfHeaderFor('POST', '/instances')).toBeNull();
  });
});

describe('ApiError', () => {
  it('carries code, status and details', () => {
    const error = new ApiError(409, 'conflict', 'already exists', { name: 'x' });
    expect(error).toBeInstanceOf(Error);
    expect(error.status).toBe(409);
    expect(error.code).toBe('conflict');
    expect(error.message).toBe('already exists');
    expect(error.details).toEqual({ name: 'x' });
  });
});
