import { http } from './client';
import type {
  HomeAssistantConfigRequest,
  HomeAssistantEntitiesResponse,
  HomeAssistantProvisionResponse,
  HomeAssistantScene,
  HomeAssistantStatus,
} from './types';

function isScene(domain: string): boolean {
  return domain === 'scene';
}

/** Home Assistant provisioning, entities and scenes. */
export const homeAssistantApi = {
  status: (): Promise<HomeAssistantStatus> =>
    http.get<HomeAssistantStatus>('/homeassistant/status'),

  provision: (): Promise<HomeAssistantProvisionResponse> =>
    http.post<HomeAssistantProvisionResponse>('/homeassistant/provision'),

  configure: (payload: HomeAssistantConfigRequest): Promise<void> =>
    http.put<void>('/homeassistant/config', payload),

  entities: (): Promise<HomeAssistantEntitiesResponse> =>
    http.get<HomeAssistantEntitiesResponse>('/homeassistant/entities'),

  callService: (domain: string, service: string, data: Record<string, unknown>): Promise<void> =>
    http.post<void>(
      `/homeassistant/services/${encodeURIComponent(domain)}/${encodeURIComponent(service)}`,
      { data },
    ),

  /** Scenes are entities in the `scene` domain. */
  scenes: async (): Promise<HomeAssistantScene[]> => {
    const { entities } = await homeAssistantApi.entities();
    return entities
      .filter((entity) => isScene(entity.domain))
      .map((entity) => ({ entity_id: entity.entity_id, name: entity.name }));
  },
};
