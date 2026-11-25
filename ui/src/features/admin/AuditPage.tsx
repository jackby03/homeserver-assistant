import { useMemo, useState } from 'react';
import { RefreshCw, ScrollText, Search } from 'lucide-react';
import { systemApi } from '@/api/system';
import type { AuditEntry, AuditOutcome } from '@/api/types';
import { DataTable } from '@/components/Table';
import type { Column } from '@/components/Table';
import { EmptyState } from '@/components/EmptyState';
import { Modal } from '@/components/Modal';
import { PageHeader } from '@/components/PageHeader';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { formatDateTime } from '@/lib/format';
import { useResource } from '@/lib/use-resource';

const OUTCOMES: AuditOutcome[] = ['success', 'denied', 'failure'];

const OUTCOME_KEYS: Record<AuditOutcome, DictKey> = {
  success: 'audit.outcome.success',
  denied: 'audit.outcome.denied',
  failure: 'audit.outcome.failure',
};

type TimeRange = 'all' | '1h' | '24h' | '7d';

const RANGE_MS: Record<Exclude<TimeRange, 'all'>, number> = {
  '1h': 3_600_000,
  '24h': 86_400_000,
  '7d': 7 * 86_400_000,
};

export function AuditPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const [refreshToken, setRefreshToken] = useState(0);
  const [actor, setActor] = useState('all');
  const [outcome, setOutcome] = useState<'all' | AuditOutcome>('all');
  const [range, setRange] = useState<TimeRange>('all');
  const [search, setSearch] = useState('');
  const [detail, setDetail] = useState<AuditEntry | null>(null);

  const audit = useResource(() => systemApi.audit(200), [refreshToken]);
  const entries = audit.data ?? [];

  const actors = useMemo(() => {
    const values = new Set(entries.map((entry) => entry.actor));
    return [...values].sort((a, b) => a.localeCompare(b));
  }, [entries]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    const now = Date.now();
    return entries.filter((entry) => {
      if (actor !== 'all' && entry.actor !== actor) return false;
      if (outcome !== 'all' && entry.outcome !== outcome) return false;
      if (range !== 'all') {
        const timestamp = new Date(entry.timestamp).getTime();
        if (!Number.isFinite(timestamp) || now - timestamp > RANGE_MS[range]) return false;
      }
      if (needle !== '') {
        const haystack = `${entry.action} ${entry.target ?? ''} ${entry.actor}`.toLocaleLowerCase();
        if (!haystack.includes(needle)) return false;
      }
      return true;
    });
  }, [entries, actor, outcome, range, search]);

  const columns: Column<AuditEntry>[] = [
    {
      key: 'seq',
      header: t('audit.seq'),
      render: (entry) => <span className="font-mono text-[11px] tabular text-ink-500">{entry.seq}</span>,
    },
    {
      key: 'time',
      header: t('audit.time'),
      render: (entry) => (
        <span className="whitespace-nowrap font-mono text-[11px] tabular text-ink-400">
          {formatDateTime(entry.timestamp, language, { dateStyle: 'short', timeStyle: 'medium' })}
        </span>
      ),
    },
    {
      key: 'actor',
      header: t('audit.actor'),
      render: (entry) => <span className="text-[12.5px] text-ink-200">{entry.actor}</span>,
    },
    {
      key: 'action',
      header: t('audit.action'),
      render: (entry) => (
        <span className="font-mono text-[11.5px] text-mint-300/90">{entry.action}</span>
      ),
    },
    {
      key: 'target',
      header: t('audit.target'),
      render: (entry) =>
        entry.target !== null ? (
          <span className="block max-w-[180px] truncate font-mono text-[11px] text-ink-400">{entry.target}</span>
        ) : (
          <span className="text-ink-600">—</span>
        ),
    },
    {
      key: 'ip',
      header: t('audit.ip'),
      render: (entry) => (
        <span className="font-mono text-[11px] text-ink-500">{entry.ip ?? '—'}</span>
      ),
    },
    {
      key: 'outcome',
      header: t('audit.outcome'),
      align: 'right',
      render: (entry) => <StatusPill status={entry.outcome} />,
    },
  ];

  const selectClass =
    'min-h-[40px] rounded-xl border border-white/[0.09] bg-night-800/70 px-3 text-[12.5px] text-ink-200 focus:border-mint-400/60 focus:outline-none';

  return (
    <div className="space-y-4">
      <PageHeader
        title={t('audit.title')}
        subtitle={t('audit.subtitle')}
        actions={
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => setRefreshToken((value) => value + 1)}>
            <RefreshCw size={14} className={audit.refreshing ? 'animate-spin' : undefined} />
            {t('common.refresh')}
          </button>
        }
      />

      <div className="glass flex flex-wrap items-center gap-2 p-3">
        <div className="relative min-w-[180px] flex-1">
          <Search size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-ink-500" />
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t('audit.search')}
            aria-label={t('audit.search')}
            className="input min-h-[40px] w-full pl-9"
          />
        </div>
        <select
          className={selectClass}
          value={actor}
          onChange={(event) => setActor(event.target.value)}
          aria-label={t('audit.actor')}
        >
          <option value="all">{t('audit.allActors')}</option>
          {actors.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
        <select
          className={selectClass}
          value={outcome}
          onChange={(event) => setOutcome(event.target.value as 'all' | AuditOutcome)}
          aria-label={t('audit.outcome')}
        >
          <option value="all">{t('audit.allOutcomes')}</option>
          {OUTCOMES.map((value) => (
            <option key={value} value={value}>
              {t(OUTCOME_KEYS[value])}
            </option>
          ))}
        </select>
        <select
          className={selectClass}
          value={range}
          onChange={(event) => setRange(event.target.value as TimeRange)}
          aria-label={t('audit.time')}
        >
          <option value="all">{t('common.all')}</option>
          <option value="1h">1 h</option>
          <option value="24h">24 h</option>
          <option value="7d">7 d</option>
        </select>
      </div>

      {audit.error !== null && entries.length === 0 ? (
        <div className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3 text-sm text-coral-200">
          {t(errorMessageKey(audit.error))}
        </div>
      ) : null}

      <p className="text-[11px] text-ink-500">{t('audit.limit', { count: filtered.length })}</p>

      <DataTable
        columns={columns}
        rows={filtered}
        rowKey={(entry) => String(entry.seq)}
        loading={audit.loading}
        dense
        onRowClick={(entry) => setDetail(entry)}
        empty={
          <EmptyState
            icon={<ScrollText size={20} />}
            title={t('audit.empty')}
            body={t('audit.subtitle')}
          />
        }
      />

      <Modal
        open={detail !== null}
        onClose={() => setDetail(null)}
        size="md"
        title={detail !== null ? `${t('audit.seq')}${detail.seq} · ${detail.action}` : ''}
        description={detail !== null ? formatDateTime(detail.timestamp, language) : undefined}
      >
        {detail !== null ? (
          <div className="space-y-3 text-[12.5px]">
            <div className="grid gap-2 sm:grid-cols-2">
              <InfoRow label={t('audit.actor')} value={detail.actor} />
              <InfoRow label={t('audit.outcome')} value={t(OUTCOME_KEYS[detail.outcome])} />
              <InfoRow label={t('audit.target')} value={detail.target ?? '—'} />
              <InfoRow label={t('audit.ip')} value={detail.ip ?? '—'} />
            </div>
            <div>
              <p className="field-label">{t('audit.details')}</p>
              <pre className="max-h-64 overflow-auto rounded-xl border border-white/[0.06] bg-[#040810] p-3 font-mono text-[10.5px] leading-relaxed text-ink-300">
                {detail.details !== null && detail.details !== undefined
                  ? JSON.stringify(detail.details, null, 2)
                  : t('audit.noDetails')}
              </pre>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="min-w-0 rounded-xl border border-white/[0.05] bg-white/[0.02] px-3 py-2">
      <p className="text-[10px] uppercase tracking-wider text-ink-500">{label}</p>
      <p className="mt-0.5 truncate font-mono text-[11.5px] text-ink-200">{value}</p>
    </div>
  );
}
