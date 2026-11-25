import { http, setCsrfToken } from './client';
import type {
  HealthResponse,
  LoginRequest,
  SessionResponse,
  SetupStatus,
} from './types';

/** Public and session endpoints. */
export const authApi = {
  health: (): Promise<HealthResponse> => http.get<HealthResponse>('/health'),

  /** Public: tells the SPA whether first boot is complete. */
  setupStatus: (): Promise<SetupStatus> => http.get<SetupStatus>('/setup/status'),

  async login(payload: LoginRequest): Promise<SessionResponse> {
    const session = await http.post<SessionResponse>('/auth/login', payload);
    setCsrfToken(session.csrf_token);
    return session;
  },

  async logout(): Promise<void> {
    try {
      await http.post<void>('/auth/logout');
    } finally {
      setCsrfToken(null);
    }
  },

  /** Current session; silent variant is used for background probes. */
  async me(options: { silent?: boolean } = {}): Promise<SessionResponse> {
    const session = await http.get<SessionResponse>('/auth/me', {
      skipUnauthorizedEvent: options.silent === true,
    });
    setCsrfToken(session.csrf_token);
    return session;
  },
};
