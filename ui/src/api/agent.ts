import { http } from './client';
import type {
  AgentChatRequest,
  AgentChatResponse,
  CreateProviderRequest,
  ProviderRecord,
  ProviderTestResult,
  ProvidersResponse,
  ToolDefinition,
  ToolsResponse,
  UpsertWorkflowRequest,
  Workflow,
  WorkflowRunRequest,
  WorkflowRunResponse,
  WorkflowsResponse,
} from './types';
import { normalizeTools } from './types';

/** LLM providers, tool registry, chat testing and workflows. */
export const agentApi = {
  providers: (): Promise<ProvidersResponse> => http.get<ProvidersResponse>('/agent/providers'),

  createProvider: (payload: CreateProviderRequest): Promise<ProviderRecord> =>
    http.post<ProviderRecord>('/agent/providers', payload),

  testProvider: (id: string): Promise<ProviderTestResult> =>
    http.post<ProviderTestResult>(`/agent/providers/${encodeURIComponent(id)}/test`),

  deleteProvider: (id: string): Promise<void> =>
    http.delete<void>(`/agent/providers/${encodeURIComponent(id)}`),

  activateProvider: (id: string): Promise<void> =>
    http.post<void>(`/agent/providers/${encodeURIComponent(id)}/activate`),

  tools: async (): Promise<ToolDefinition[]> => {
    const response = await http.get<ToolDefinition[] | ToolsResponse>('/agent/tools');
    return normalizeTools(response);
  },

  chat: (payload: AgentChatRequest): Promise<AgentChatResponse> =>
    http.post<AgentChatResponse>('/agent/chat', payload),

  workflows: (): Promise<WorkflowsResponse> => http.get<WorkflowsResponse>('/agent/workflows'),

  createWorkflow: (payload: UpsertWorkflowRequest): Promise<Workflow> =>
    http.post<Workflow>('/agent/workflows', payload),

  updateWorkflow: (id: string, payload: UpsertWorkflowRequest): Promise<Workflow> =>
    http.put<Workflow>(`/agent/workflows/${encodeURIComponent(id)}`, payload),

  deleteWorkflow: (id: string): Promise<void> =>
    http.delete<void>(`/agent/workflows/${encodeURIComponent(id)}`),

  runWorkflow: (id: string, payload: WorkflowRunRequest): Promise<WorkflowRunResponse> =>
    http.post<WorkflowRunResponse>(`/agent/workflows/${encodeURIComponent(id)}/run`, payload),
};

/** Tool descriptor lookup used by the permissions matrix. */
export function toolNameSet(tools: ToolDefinition[]): Set<string> {
  return new Set(tools.map((tool) => tool.name));
}
