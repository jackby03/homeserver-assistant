import { http } from './client';
import type {
  CreateInstanceRequest,
  CreateSnapshotRequest,
  InstanceListResponse,
  InstanceLogsResponse,
  InstanceSummary,
  SnapshotListResponse,
  UpdateInstanceLimitsRequest,
} from './types';

/** Container and VM management through the runtime provider. */
export const instancesApi = {
  list: (): Promise<InstanceListResponse> => http.get<InstanceListResponse>('/instances'),

  get: (name: string): Promise<InstanceSummary> =>
    http.get<InstanceSummary>(`/instances/${encodeURIComponent(name)}`),

  create: (payload: CreateInstanceRequest): Promise<InstanceSummary> =>
    http.post<InstanceSummary>('/instances', payload),

  start: (name: string): Promise<void> =>
    http.post<void>(`/instances/${encodeURIComponent(name)}/start`),

  stop: (name: string): Promise<void> =>
    http.post<void>(`/instances/${encodeURIComponent(name)}/stop`),

  restart: (name: string): Promise<void> =>
    http.post<void>(`/instances/${encodeURIComponent(name)}/restart`),

  remove: (name: string): Promise<void> =>
    http.delete<void>(`/instances/${encodeURIComponent(name)}`),

  updateLimits: (name: string, payload: UpdateInstanceLimitsRequest): Promise<void> =>
    http.patch<void>(`/instances/${encodeURIComponent(name)}/limits`, payload),

  snapshots: (name: string): Promise<SnapshotListResponse> =>
    http.get<SnapshotListResponse>(`/instances/${encodeURIComponent(name)}/snapshots`),

  createSnapshot: (name: string, payload: CreateSnapshotRequest): Promise<void> =>
    http.post<void>(`/instances/${encodeURIComponent(name)}/snapshots`, payload),

  restoreSnapshot: (name: string, snapshot: string): Promise<void> =>
    http.post<void>(
      `/instances/${encodeURIComponent(name)}/snapshots/${encodeURIComponent(snapshot)}/restore`,
    ),

  deleteSnapshot: (name: string, snapshot: string): Promise<void> =>
    http.delete<void>(
      `/instances/${encodeURIComponent(name)}/snapshots/${encodeURIComponent(snapshot)}`,
    ),

  logs: (name: string, lines = 200): Promise<InstanceLogsResponse> =>
    http.get<InstanceLogsResponse>(
      `/instances/${encodeURIComponent(name)}/logs?lines=${encodeURIComponent(String(lines))}`,
    ),
};
