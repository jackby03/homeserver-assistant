import { ArrowUpRight, ShieldCheck, Wrench } from 'lucide-react';
import { Link } from 'react-router-dom';
import type { ToolDefinition } from '@/api/types';
import { EmptyState } from '@/components/EmptyState';
import { useI18n } from '@/i18n/I18nProvider';

interface ToolsSectionProps {
  tools: ToolDefinition[];
  loading: boolean;
}

/** Read-only reference of the tools the agent may call. */
export function ToolsSection({ tools, loading }: ToolsSectionProps): React.JSX.Element {
  const { t } = useI18n();

  return (
    <section className="glass space-y-4 p-5 sm:p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 place-items-center rounded-xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
            <Wrench size={16} />
          </span>
          <div>
            <h2 className="font-display text-[15px] font-medium text-ink-100">{t('agent.tools.title')}</h2>
            <p className="text-[11px] text-ink-500">{t('agent.tools.subtitle')}</p>
          </div>
        </div>
        <Link
          to="/admin/profiles"
          className="inline-flex min-h-[36px] items-center gap-1.5 rounded-lg border border-white/[0.08] px-3 text-[12px] text-ink-300 transition hover:border-white/[0.16] hover:text-ink-100"
        >
          <ShieldCheck size={13} className="text-mint-400" />
          {t('agent.tools.permissionsTitle')}
          <ArrowUpRight size={13} />
        </Link>
      </div>

      {loading ? (
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {Array.from({ length: 6 }).map((_, index) => (
            <div key={index} className="skeleton h-20 w-full rounded-xl" />
          ))}
        </div>
      ) : tools.length === 0 ? (
        <EmptyState title={t('agent.tools.empty')} />
      ) : (
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {tools.map((tool) => (
            <details
              key={tool.name}
              className="group rounded-xl border border-white/[0.06] bg-white/[0.015] px-3.5 py-2.5 open:border-white/[0.12]"
            >
              <summary className="flex cursor-pointer list-none items-start gap-2 text-[12.5px]">
                <span
                  aria-hidden="true"
                  className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-mint-400/70 transition group-open:bg-mint-400"
                />
                <span className="min-w-0">
                  <span className="block truncate font-mono text-[12px] text-ink-100">{tool.name}</span>
                  <span className="mt-0.5 line-clamp-2 block text-[10.5px] leading-relaxed text-ink-500">
                    {tool.description}
                  </span>
                </span>
              </summary>
              <pre className="mt-2 max-h-40 overflow-auto rounded-lg bg-[#040810] p-2.5 font-mono text-[10px] leading-relaxed text-ink-400">
                {JSON.stringify(tool.parameters, null, 2)}
              </pre>
            </details>
          ))}
        </div>
      )}
      <p className="text-[11px] leading-relaxed text-ink-500">{t('agent.tools.permissionsHint')}</p>
    </section>
  );
}
