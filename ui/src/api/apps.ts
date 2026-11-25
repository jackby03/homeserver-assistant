import { http } from './client';
import type {
  AppCatalogResponse,
  AppsResponse,
  AppRecord,
  AppTemplate,
  DeployAppRequest,
} from './types';
import { normalizeCatalog } from './types';

/** App catalog and deployed application lifecycle. */
export const appsApi = {
  /** Current daemon returns a bare template array; older builds wrapped it. */
  catalog: async (): Promise<AppTemplate[]> => {
    const response = await http.get<AppTemplate[] | AppCatalogResponse>('/apps/catalog');
    return normalizeCatalog(response);
  },

  list: (): Promise<AppsResponse> => http.get<AppsResponse>('/apps'),

  deploy: (payload: DeployAppRequest): Promise<AppRecord> =>
    http.post<AppRecord>('/apps', payload),

  start: (id: string): Promise<void> => http.post<void>(`/apps/${encodeURIComponent(id)}/start`),

  stop: (id: string): Promise<void> => http.post<void>(`/apps/${encodeURIComponent(id)}/stop`),

  remove: (id: string): Promise<void> => http.delete<void>(`/apps/${encodeURIComponent(id)}`),
};
