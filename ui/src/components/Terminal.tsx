import { useCallback, useEffect, useRef, useState } from 'react';
import { Eraser, TerminalSquare } from 'lucide-react';
import { ReconnectingSocket } from '@/api/ws';
import type { SocketStatus } from '@/api/ws';
import type { ExecServerMessage } from '@/api/types';
import { useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';

interface Line {
  id: number;
  kind: 'out' | 'in' | 'sys';
  text: string;
}

interface TerminalProps {
  instanceName: string;
  className?: string;
}

const MAX_LINES = 2_000;

/**
 * Interactive console attached to `WS /instances/:name/exec`.
 * Client sends `{type:"input",data}` frames; server answers with output/exit.
 */
export function Terminal({ instanceName, className }: TerminalProps): React.JSX.Element {
  const { t } = useI18n();
  const [lines, setLines] = useState<Line[]>([]);
  const [status, setStatus] = useState<SocketStatus>('connecting');
  const [input, setInput] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const nextLineId = useRef(1);
  const socketRef = useRef<ReconnectingSocket<ExecServerMessage> | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const appendLine = useCallback((kind: Line['kind'], text: string) => {
    const id = nextLineId.current;
    nextLineId.current += 1;
    setLines((current) => {
      const next = [...current, { id, kind, text }];
      return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next;
    });
  }, []);

  useEffect(() => {
    const socket = new ReconnectingSocket<ExecServerMessage>(
      `/instances/${encodeURIComponent(instanceName)}/exec`,
      {
        onStatus: (nextStatus) => setStatus(nextStatus),
        onMessage: (message) => {
          if (message.type === 'output') {
            appendLine('out', message.data);
          } else if (message.type === 'exit') {
            appendLine('sys', t('inst.terminal.exited', { code: message.code }));
          }
        },
      },
    );
    socketRef.current = socket;
    socket.connect();
    return () => {
      socket.close();
      socketRef.current = null;
    };
  }, [instanceName, appendLine, t]);

  useEffect(() => {
    const node = scrollRef.current;
    if (node !== null) node.scrollTop = node.scrollHeight;
  }, [lines]);

  const submit = (): void => {
    const command = input;
    if (command.trim() === '') return;
    appendLine('in', command);
    socketRef.current?.send({ type: 'input', data: `${command}\n` });
    setHistory((current) => [...current.slice(-49), command]);
    setHistoryIndex(null);
    setInput('');
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (history.length === 0) return;
      const nextIndex = historyIndex === null ? history.length - 1 : Math.max(0, historyIndex - 1);
      setHistoryIndex(nextIndex);
      setInput(history[nextIndex] ?? '');
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (historyIndex === null) return;
      if (historyIndex >= history.length - 1) {
        setHistoryIndex(null);
        setInput('');
      } else {
        const nextIndex = historyIndex + 1;
        setHistoryIndex(nextIndex);
        setInput(history[nextIndex] ?? '');
      }
    }
  };

  const statusLabel = status === 'open' ? t('common.connected') : status === 'closed' ? t('common.disconnected') : t('common.reconnecting');

  return (
    <div
      className={cn(
        'flex min-h-[320px] flex-col overflow-hidden rounded-2xl border border-white/[0.07] bg-[#040810]',
        className,
      )}
    >
      <header className="flex items-center justify-between gap-3 border-b border-white/[0.06] bg-white/[0.02] px-4 py-2.5">
        <div className="flex items-center gap-2.5 text-[12px] text-ink-300">
          <span className="flex gap-1.5" aria-hidden="true">
            <span className="h-2 w-2 rounded-full bg-coral-400/70" />
            <span className="h-2 w-2 rounded-full bg-amber-400/70" />
            <span className="h-2 w-2 rounded-full bg-mint-500/70" />
          </span>
          <TerminalSquare size={14} className="text-ink-500" />
          <span className="font-medium">{t('inst.terminal.title')}</span>
          <span className="text-ink-500">{instanceName}</span>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={cn(
              'inline-flex items-center gap-1.5 text-[11px]',
              status === 'open' ? 'text-mint-400' : 'text-amber-300',
            )}
          >
            <span
              className={cn(
                'h-1.5 w-1.5 rounded-full',
                status === 'open' ? 'bg-mint-500' : 'bg-amber-400 animate-pulse',
              )}
            />
            {statusLabel}
          </span>
          <button
            type="button"
            className="inline-flex min-h-[32px] items-center gap-1.5 rounded-lg px-2 text-[11px] text-ink-400 transition hover:bg-white/[0.06] hover:text-ink-100"
            onClick={() => setLines([])}
          >
            <Eraser size={13} />
            {t('inst.terminal.clear')}
          </button>
        </div>
      </header>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto px-4 py-3 font-mono text-[12.5px] leading-relaxed"
        onClick={() => inputRef.current?.focus()}
        role="log"
        aria-live="polite"
      >
        {status === 'connecting' && lines.length === 0 ? (
          <p className="text-ink-500">{t('inst.terminal.connecting')}</p>
        ) : null}
        {lines.map((line) => (
          <pre
            key={line.id}
            className={cn(
              'whitespace-pre-wrap break-words',
              line.kind === 'in' ? 'text-mint-300' : line.kind === 'sys' ? 'text-amber-300/90' : 'text-ink-200',
            )}
          >
            {line.kind === 'in' ? `❯ ${line.text}` : line.text}
          </pre>
        ))}
      </div>

      <form
        className="flex items-center gap-2 border-t border-white/[0.06] bg-white/[0.015] px-4 py-2.5"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <span className="font-mono text-[13px] text-mint-400" aria-hidden="true">
          ❯
        </span>
        <input
          ref={inputRef}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={onKeyDown}
          spellCheck={false}
          autoComplete="off"
          autoCapitalize="off"
          placeholder={t('inst.terminal.inputHint')}
          aria-label={t('inst.terminal.inputHint')}
          className="min-h-[36px] flex-1 border-none bg-transparent font-mono text-[13px] text-ink-100 placeholder:text-ink-600 focus:outline-none"
        />
        <button
          type="submit"
          className="rounded-lg border border-white/10 px-2.5 py-1 text-[11px] text-ink-300 transition hover:bg-white/[0.06] hover:text-ink-50 disabled:opacity-40"
          disabled={input.trim() === ''}
        >
          ↵
        </button>
      </form>
    </div>
  );
}
