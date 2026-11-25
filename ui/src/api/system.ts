import { http } from './client';
import type {
  AuditEntry,
  SystemOverview,
  SystemVersion,
} from './types';

/**
 * `GET /audit` currently returns a bare array. Older builds wrapped it in
 * `{entries}`; accept both so a daemon upgrade never breaks the dashboard.
 */
async function fetchAudit(limit: number): Promise<AuditEntry[]> {
  const response = await http.get<AuditEntry[] | { entries: AuditEntry[] }>(
    `/audit?limit=${encodeURIComponent(String(limit))}`,
  );
  return Array.isArray(response) ? response : response.entries;
}

/** Host metrics, version and audit log. */
export const systemApi = {
  overview: (signal?: AbortSignal): Promise<SystemOverview> =>
    http.get<SystemOverview>('/system/overview', signal ? { signal } : undefined),

  version: (): Promise<SystemVersion> => http.get<SystemVersion>('/system/version'),

  audit: (limit = 50): Promise<AuditEntry[]> => fetchAudit(limit),
};
