import { useRef, useState } from 'react';
import { Bot, Send, Sparkles, Trash2, UserRound, Wrench } from 'lucide-react';
import { agentApi } from '@/api/agent';
import type {
  AgentChatResponse,
  ChatMessage,
  Profile,
  ProviderRecord,
  ToolTraceEntry,
} from '@/api/types';
import { Field } from '@/components/Field';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { useToast } from '@/state/ToastProvider';

interface TranscriptEntry {
  id: number;
  role: 'user' | 'assistant';
  content: string;
  trace?: ToolTraceEntry[];
  usage?: AgentChatResponse['usage'];
  model?: string;
}

interface ChatTesterProps {
  profiles: Profile[];
  providers: ProviderRecord[];
}

/** Agent playground: chat with the active provider and inspect tool calls. */
export function ChatTester({ profiles, providers }: ChatTesterProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [profileId, setProfileId] = useState(profiles[0]?.id ?? '');
  const [useTools, setUseTools] = useState(true);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [transcript, setTranscript] = useState<TranscriptEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const nextId = useRef(1);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const effectiveProfileId = profiles.some((profile) => profile.id === profileId)
    ? profileId
    : (profiles[0]?.id ?? '');

  const activeProvider = providers.find((provider) => provider.enabled) ?? providers[0] ?? null;

  const append = (entry: Omit<TranscriptEntry, 'id'>): void => {
    const id = nextId.current;
    nextId.current += 1;
    setTranscript((current) => [...current, { ...entry, id }]);
    window.setTimeout(() => {
      const node = scrollRef.current;
      if (node !== null) node.scrollTop = node.scrollHeight;
    }, 30);
  };

  const send = async (): Promise<void> => {
    const text = input.trim();
    if (text === '' || busy) return;
    if (effectiveProfileId === '') {
      setError(t('agent.chat.selectProfile'));
      return;
    }
    setError(null);
    setInput('');
    append({ role: 'user', content: text });
    const history: ChatMessage[] = [
      ...transcript.map((entry) => ({ role: entry.role, content: entry.content })),
      { role: 'user', content: text },
    ];
    setBusy(true);
    try {
      const response = await agentApi.chat({
        profile_id: effectiveProfileId,
        messages: history,
        use_tools: useTools,
      });
      append({
        role: 'assistant',
        content: response.reply,
        trace: response.tool_trace,
        usage: response.usage,
        model: response.model,
      });
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="glass flex min-h-[480px] flex-col p-5 sm:p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 place-items-center rounded-xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
            <Sparkles size={16} />
          </span>
          <div>
            <h2 className="font-display text-[15px] font-medium text-ink-100">{t('agent.chat.title')}</h2>
            <p className="text-[11px] text-ink-500">{t('agent.chat.subtitle')}</p>
          </div>
        </div>
        <button
          type="button"
          className="btn btn-ghost btn-sm text-ink-500"
          onClick={() => setTranscript([])}
          disabled={busy || transcript.length === 0}
        >
          <Trash2 size={13} />
          {t('agent.chat.empty')}
        </button>
      </div>

      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <Field label={t('agent.chat.selectProfile')} htmlFor="chat-profile">
          <select
            id="chat-profile"
            className="input"
            value={effectiveProfileId}
            onChange={(event) => setProfileId(event.target.value)}
          >
            {profiles.length === 0 ? <option value="">{t('agent.workflows.noProfiles')}</option> : null}
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name}
              </option>
            ))}
          </select>
        </Field>
        <div className="flex items-end pb-1">
          <label className="flex min-h-[44px] cursor-pointer items-center gap-2.5 text-[12.5px] text-ink-300">
            <input
              type="checkbox"
              checked={useTools}
              onChange={(event) => setUseTools(event.target.checked)}
              className="h-4 w-4 accent-[#34d399]"
            />
            <Wrench size={14} className="text-mint-400" />
            {t('agent.chat.useTools')}
            {activeProvider !== null ? (
              <span className="chip ml-1 font-mono">{activeProvider.model}</span>
            ) : null}
          </label>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="mt-4 min-h-[200px] flex-1 space-y-3 overflow-y-auto rounded-2xl border border-white/[0.05] bg-night-950/40 p-3.5"
        style={{ maxHeight: 420 }}
        aria-live="polite"
      >
        {transcript.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 py-10 text-center">
            <Bot size={22} className="text-ink-600" />
            <p className="text-[12.5px] text-ink-500">{t('agent.chat.empty')}</p>
          </div>
        ) : (
          transcript.map((entry) => (
            <div
              key={entry.id}
              className={cn(
                'flex gap-2.5',
                entry.role === 'user' ? 'flex-row-reverse text-right' : 'flex-row',
              )}
            >
              <span
                className={cn(
                  'mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-full border',
                  entry.role === 'user'
                    ? 'border-white/[0.09] bg-white/[0.04] text-ink-300'
                    : 'border-mint-400/25 bg-mint-400/[0.08] text-mint-300',
                )}
              >
                {entry.role === 'user' ? <UserRound size={13} /> : <Bot size={13} />}
              </span>
              <div className={cn('min-w-0 max-w-[86%] space-y-2', entry.role === 'user' && 'items-end')}>
                <div
                  className={cn(
                    'inline-block whitespace-pre-wrap rounded-2xl border px-3.5 py-2.5 text-[12.5px] leading-relaxed',
                    entry.role === 'user'
                      ? 'border-white/[0.08] bg-white/[0.04] text-ink-100'
                      : 'border-mint-400/15 bg-mint-400/[0.035] text-ink-200',
                  )}
                >
                  {entry.content}
                </div>
                {entry.trace !== undefined && entry.trace.length > 0 ? (
                  <div className="space-y-1.5 text-left">
                    <p className="text-[10px] uppercase tracking-wider text-ink-500">
                      {t('agent.chat.trace')}
                    </p>
                    {entry.trace.map((trace, index) => (
                      <TraceRow key={`${entry.id}-${index}`} trace={trace} />
                    ))}
                  </div>
                ) : null}
                {entry.usage !== undefined ? (
                  <p className="text-[10px] tabular text-ink-600">
                    {entry.model ?? ''} · {t('agent.chat.tokens', {
                      prompt: entry.usage.prompt_tokens,
                      completion: entry.usage.completion_tokens,
                    })}
                  </p>
                ) : null}
              </div>
            </div>
          ))
        )}
        {busy ? (
          <div className="flex items-center gap-2 text-[12px] text-mint-300">
            <Spinner size={14} />
            {t('agent.chat.sending')}
          </div>
        ) : null}
      </div>

      {error !== null ? (
        <p role="alert" className="mt-3 rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[12.5px] text-coral-300">
          {error}
        </p>
      ) : null}

      <form
        className="mt-3 flex items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
        <textarea
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={t('agent.chat.placeholder')}
          aria-label={t('agent.chat.placeholder')}
          rows={2}
          className="input min-h-[52px] flex-1 resize-none"
        />
        <button
          type="submit"
          className="btn btn-primary mb-0.5"
          disabled={busy || input.trim() === ''}
          aria-label={t('agent.chat.send')}
        >
          {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Send size={16} />}
        </button>
      </form>
      {profiles.length === 0 ? (
        <p className="mt-2 text-[11px] text-amber-300/80">{t('agent.workflows.noProfiles')}</p>
      ) : null}
    </section>
  );
}

function TraceRow({ trace }: { trace: ToolTraceEntry }): React.JSX.Element {
  const { t } = useI18n();
  return (
    <div
      className={cn(
        'rounded-xl border px-2.5 py-2',
        trace.allowed
          ? 'border-white/[0.07] bg-white/[0.02]'
          : 'border-coral-400/20 bg-coral-500/[0.05]',
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-[11px] text-ink-200">{trace.tool}</span>
        <StatusPill
          label={trace.allowed ? t('agent.chat.allowed') : t('agent.chat.denied')}
          tone={trace.allowed ? 'success' : 'danger'}
        />
        <span className="ml-auto text-[10px] tabular text-ink-600">
          {t('agent.chat.duration', { ms: trace.duration_ms })}
        </span>
      </div>
      {trace.error !== null ? (
        <p className="mt-1 text-[10.5px] text-coral-300">{trace.error}</p>
      ) : trace.output !== null ? (
        <p className="mt-1 line-clamp-2 font-mono text-[10px] leading-relaxed text-ink-500">
          {trace.output}
        </p>
      ) : null}
    </div>
  );
}
