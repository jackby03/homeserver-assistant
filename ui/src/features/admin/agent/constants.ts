import type { LlmProviderKind } from '@/api/types';
import type { DictKey } from '@/i18n/dictionary';

export const PROVIDER_KINDS: LlmProviderKind[] = [
  'openai',
  'anthropic',
  'deepseek',
  'opencode_go',
  'custom',
];

export const PROVIDER_LABEL_KEYS: Record<LlmProviderKind, DictKey> = {
  openai: 'agent.kind.openai',
  anthropic: 'agent.kind.anthropic',
  deepseek: 'agent.kind.deepseek',
  opencode_go: 'agent.kind.opencode_go',
  custom: 'agent.kind.custom',
};

export const PROVIDER_HINT_KEYS: Record<LlmProviderKind, DictKey> = {
  openai: 'agent.kindHint.openai',
  anthropic: 'agent.kindHint.anthropic',
  deepseek: 'agent.kindHint.deepseek',
  opencode_go: 'agent.kindHint.opencode_go',
  custom: 'agent.kindHint.custom',
};

export const BASE_URL_PLACEHOLDERS: Record<LlmProviderKind, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  deepseek: 'https://api.deepseek.com/v1',
  opencode_go: 'https://opencode.example.com/v1',
  custom: 'https://llm.example.com/v1',
};

export const MODEL_PLACEHOLDERS: Record<LlmProviderKind, string> = {
  openai: 'gpt-4.1-mini',
  anthropic: 'claude-sonnet-4',
  deepseek: 'deepseek-chat',
  opencode_go: 'opencode-go/assistant',
  custom: 'my-model',
};
