import { http } from './client';
import type {
  CreateProfileRequest,
  FaceEnrollRequest,
  Profile,
  ProfilesResponse,
  UpdateProfileRequest,
} from './types';
import { normalizeProfiles } from './types';

/** Family profiles, avatars and local face enrollment. */
export const profilesApi = {
  /** Current daemon returns a bare profile array; older builds wrapped it. */
  list: async (): Promise<Profile[]> => {
    const response = await http.get<Profile[] | ProfilesResponse>('/profiles');
    return normalizeProfiles(response);
  },

  create: (payload: CreateProfileRequest): Promise<Profile> =>
    http.post<Profile>('/profiles', payload),

  update: (id: string, payload: UpdateProfileRequest): Promise<Profile> =>
    http.put<Profile>(`/profiles/${encodeURIComponent(id)}`, payload),

  remove: (id: string): Promise<void> =>
    http.delete<void>(`/profiles/${encodeURIComponent(id)}`),

  enroll: (id: string, payload: FaceEnrollRequest): Promise<void> =>
    http.post<void>(`/profiles/${encodeURIComponent(id)}/enroll`, payload),

  deleteFace: (id: string): Promise<void> =>
    http.delete<void>(`/profiles/${encodeURIComponent(id)}/face`),
};
