import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import {
  ExternalLink,
  FileSpreadsheet,
  HardDrive,
  LayoutGrid,
  Music2,
  Network,
  Package,
  Play,
  Plus,
  RotateCcw,
  Search,
  Sparkles,
  Square,
  Trash2,
  Workflow,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { appsApi } from '@/api/apps';
import type {
  AppCategory,
  AppRecord,
  AppTemplate,
  AppTemplateEnv,
  DeployAppRequest,
  NetworkSegment,
} from '@/api/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { DataTable } from '@/components/Table';
import type { Column } from '@/components/Table';
import { Drawer } from '@/components/Drawer';
import { EmptyState } from '@/components/EmptyState';
import { Field, Segmented, Toggle } from '@/components/Field';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { useI18n, errorMessageKey } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { formatDateTime } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

const CATEGORY_ICONS: Record<AppCategory, LucideIcon> = {
  automation: Workflow,
  media: Music2,
  networking: Network,
  productivity: FileSpreadsheet,
  storage: HardDrive,
  other: Package,
};

const CATEGORY_LABEL_KEYS: Record<AppCategory, DictKey> = {
  automation: 'apps.categories.automation',
  media: 'apps.categories.media',
  networking: 'apps.categories.networking',
  productivity: 'apps.categories.productivity',
  storage: 'apps.categories.storage',
  other: 'apps.categories.other',
};

const CATEGORY_ORDER: AppCategory[] = [
  'automation',
  'media',
  'networking',
  'productivity',
  'storage',
  'other',
];

export function AppsPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [refreshToken, setRefreshToken] = useState(0);
  const [category, setCategory] = useState<'all' | AppCategory>('all');
  const [search, setSearch] = useState('');
  const [deployTarget, setDeployTarget] = useState<AppTemplate | null>(null);
  const [removeTarget, setRemoveTarget] = useState<AppRecord | null>(null);
  const [removing, setRemoving] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);

  const catalog = useResource(() => appsApi.catalog(), [refreshToken]);
  const apps = useResource(() => appsApi.list(), [refreshToken], { pollMs: 8_000 });

  const refresh = (): void => setRefreshToken((value) => value + 1);

  const templates = catalog.data ?? [];
  const installed = apps.data?.apps ?? [];

  const filtered = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    return templates.filter((template) => {
      if (category !== 'all' && template.category !== category) return false;
      if (needle === '') return true;
      return (
        template.name.toLocaleLowerCase().includes(needle) ||
        template.description.toLocaleLowerCase().includes(needle) ||
        template.id.toLocaleLowerCase().includes(needle)
      );
    });
  }, [templates, category, search]);

  const availableCategories = CATEGORY_ORDER.filter((item) =>
    templates.some((template) => template.category === item),
  );

  const runAction = async (app: AppRecord, action: 'start' | 'stop'): Promise<void> => {
    setPendingId(app.id);
    try {
      if (action === 'start') await appsApi.start(app.id);
      else await appsApi.stop(app.id);
      toast.push({
        level: 'success',
        message: t(action === 'start' ? 'apps.toast.started' : 'apps.toast.stopped', { name: app.name }),
      });
      apps.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setPendingId(null);
    }
  };

  const confirmRemove = async (): Promise<void> => {
    if (removeTarget === null) return;
    setRemoving(true);
    try {
      await appsApi.remove(removeTarget.id);
      toast.push({ level: 'success', message: t('apps.toast.removed', { name: removeTarget.name }) });
      setRemoveTarget(null);
      apps.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setRemoving(false);
    }
  };

  const columns: Column<AppRecord>[] = [
    {
      key: 'name',
      header: t('common.name'),
      render: (app) => {
        const template = templates.find((item) => item.id === app.template_id);
        const Icon = template !== undefined ? CATEGORY_ICONS[template.category] : Package;
        return (
          <div className="flex items-center gap-2.5">
            <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-white/[0.07] bg-white/[0.03] text-ink-300">
              <Icon size={15} />
            </span>
            <div className="min-w-0">
              <p className="truncate text-[12.5px] font-medium text-ink-100">{app.name}</p>
              <p className="truncate font-mono text-[10.5px] text-ink-500">{app.image}</p>
            </div>
          </div>
        );
      },
    },
    {
      key: 'status',
      header: t('common.status'),
      render: (app) => <StatusPill status={app.status} pulse={app.status === 'running'} />,
    },
    {
      key: 'segment',
      header: t('apps.field.segment'),
      render: (app) => (
        <StatusPill
          label={app.segment === 'dmz' ? t('apps.segmentDmz') : t('apps.segmentLan')}
          tone={app.segment === 'dmz' ? 'warning' : 'neutral'}
        />
      ),
    },
    {
      key: 'url',
      header: t('apps.field.subdomain'),
      render: (app) =>
        app.hostname !== null ? (
          <a
            href={`https://${app.hostname}`}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 font-mono text-[11.5px] text-mint-300 transition hover:text-mint-200"
            onClick={(event) => event.stopPropagation()}
          >
            {app.hostname}
            <ExternalLink size={12} />
          </a>
        ) : (
          <span className="text-[11.5px] text-ink-600">{t('apps.notPublished')}</span>
        ),
    },
    {
      key: 'created',
      header: t('inst.table.created'),
      render: (app) => (
        <span className="whitespace-nowrap text-[11.5px] text-ink-500">
          {formatDateTime(app.created_at, language, { dateStyle: 'short', timeStyle: 'short' })}
        </span>
      ),
    },
    {
      key: 'actions',
      header: t('common.actions'),
      align: 'right',
      render: (app) => (
        <div className="flex items-center justify-end gap-1" onClick={(event) => event.stopPropagation()}>
          <button
            type="button"
            className="grid h-9 w-9 place-items-center rounded-lg text-ink-400 transition hover:bg-white/[0.07] hover:text-ink-50 disabled:opacity-40"
            aria-label={app.status === 'running' ? t('apps.stop') : t('apps.start')}
            title={app.status === 'running' ? t('apps.stop') : t('apps.start')}
            disabled={pendingId === app.id || app.status === 'deploying'}
            onClick={() => void runAction(app, app.status === 'running' ? 'stop' : 'start')}
          >
            {pendingId === app.id ? (
              <Spinner size={14} />
            ) : app.status === 'running' ? (
              <Square size={14} />
            ) : (
              <Play size={14} />
            )}
          </button>
          <button
            type="button"
            className="grid h-9 w-9 place-items-center rounded-lg text-ink-400 transition hover:bg-coral-500/10 hover:text-coral-300"
            aria-label={t('apps.remove')}
            title={t('apps.remove')}
            onClick={() => setRemoveTarget(app)}
          >
            <Trash2 size={14} />
          </button>
        </div>
      ),
    },
  ];

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('apps.title')}
        subtitle={t('apps.subtitle')}
        actions={
          <button type="button" className="btn btn-secondary btn-sm" onClick={refresh} disabled={catalog.refreshing || apps.refreshing}>
            <RotateCcw size={14} className={catalog.refreshing || apps.refreshing ? 'animate-spin' : undefined} />
            {t('common.refresh')}
          </button>
        }
      />

      {/* Deployed ------------------------------------------------------------ */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="font-display text-[17px] font-medium text-ink-100">{t('apps.deployed')}</h2>
          <span className="text-[11px] text-ink-500">{installed.length}</span>
        </div>
        <DataTable
          columns={columns}
          rows={installed}
          rowKey={(app) => app.id}
          loading={apps.loading}
          empty={
            <EmptyState
              icon={<LayoutGrid size={20} />}
              title={t('apps.emptyDeployed.title')}
              body={t('apps.emptyDeployed.body')}
            />
          }
        />
      </section>

      {/* Catalog ------------------------------------------------------------- */}
      <section className="space-y-3">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <h2 className="font-display text-[17px] font-medium text-ink-100">{t('apps.catalog')}</h2>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <div className="relative">
              <Search size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-ink-500" />
              <input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t('apps.search')}
                aria-label={t('apps.search')}
                className="input min-h-[40px] w-full pl-9 sm:w-64"
              />
            </div>
          </div>
        </div>

        <div className="flex flex-wrap gap-1.5">
          <FilterChip active={category === 'all'} onClick={() => setCategory('all')}>
            {t('apps.categories.all')}
          </FilterChip>
          {availableCategories.map((item) => (
            <FilterChip key={item} active={category === item} onClick={() => setCategory(item)}>
              {t(CATEGORY_LABEL_KEYS[item])}
            </FilterChip>
          ))}
        </div>

        {catalog.loading && templates.length === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 6 }).map((_, index) => (
              <div key={index} className="skeleton h-44 w-full rounded-2xl" />
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <EmptyState title={templates.length === 0 ? t('apps.catalogEmpty') : t('apps.emptyCatalog')} />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {filtered.map((template) => (
              <CatalogCard
                key={template.id}
                template={template}
                onDeploy={() => setDeployTarget(template)}
              />
            ))}
          </div>
        )}
      </section>

      {deployTarget !== null ? (
        <DeployDrawer
          template={deployTarget}
          onClose={() => setDeployTarget(null)}
          onDeployed={() => {
            setDeployTarget(null);
            refresh();
          }}
        />
      ) : null}

      <ConfirmDialog
        open={removeTarget !== null}
        onClose={() => setRemoveTarget(null)}
        onConfirm={() => void confirmRemove()}
        busy={removing}
        title={t('apps.confirmRemove.title', { name: removeTarget?.name ?? '' })}
        message={t('apps.confirmRemove.body')}
        confirmLabel={t('apps.remove')}
      />
    </div>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        'min-h-[34px] rounded-full border px-3 text-[12px] font-medium transition duration-150',
        active
          ? 'border-mint-400/30 bg-mint-400/[0.1] text-mint-200'
          : 'border-white/[0.08] bg-white/[0.025] text-ink-400 hover:border-white/[0.16] hover:text-ink-200',
      )}
    >
      {children}
    </button>
  );
}

function CatalogCard({
  template,
  onDeploy,
}: {
  template: AppTemplate;
  onDeploy: () => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const Icon = CATEGORY_ICONS[template.category];
  return (
    <article className="glass glass-hover flex flex-col gap-3 p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span className="grid h-11 w-11 shrink-0 place-items-center rounded-xl border border-mint-400/15 bg-mint-400/[0.06] text-mint-300">
            <Icon size={19} />
          </span>
          <div className="min-w-0">
            <h3 className="truncate font-display text-[15px] font-medium text-ink-50">{template.name}</h3>
            <p className="text-[11px] text-ink-500">{t(CATEGORY_LABEL_KEYS[template.category])}</p>
          </div>
        </div>
        {template.featured ? (
          <span className="chip shrink-0 border-amber-400/25 bg-amber-400/[0.08] text-amber-300">
            <Sparkles size={11} />
            {t('apps.featured')}
          </span>
        ) : null}
      </div>
      <p className="line-clamp-2 min-h-[38px] text-[12.5px] leading-relaxed text-ink-400">
        {template.description}
      </p>
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="chip font-mono">{template.default_port}/tcp</span>
        <span className="chip">{template.segment === 'dmz' ? t('apps.segmentDmz') : t('apps.segmentLan')}</span>
      </div>
      <div className="mt-auto flex gap-2 pt-1">
        <button type="button" className="btn btn-primary btn-sm flex-1" onClick={onDeploy}>
          <Plus size={14} />
          {t('apps.deploy')}
        </button>
      </div>
    </article>
  );
}

function DeployDrawer({
  template,
  onClose,
  onDeployed,
}: {
  template: AppTemplate;
  onClose: () => void;
  onDeployed: () => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState(template.id);
  const [segment, setSegment] = useState<NetworkSegment>(template.segment);
  const [port, setPort] = useState(String(template.default_port));
  const [subdomain, setSubdomain] = useState(template.id);
  const [expose, setExpose] = useState(false);
  const [envValues, setEnvValues] = useState<Record<string, string>>(() => {
    const initial: Record<string, string> = {};
    for (const variable of template.env) {
      initial[variable.key] = variable.default ?? '';
    }
    return initial;
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (): Promise<void> => {
    setError(null);
    if (name.trim() === '') {
      setError(t('apps.validation.name'));
      return;
    }
    const internalPort = Number(port);
    if (!Number.isInteger(internalPort) || internalPort < 1 || internalPort > 65_535) {
      setError(t('apps.validation.port'));
      return;
    }
    for (const variable of template.env) {
      if (variable.required && (envValues[variable.key] ?? '').trim() === '') {
        setError(t('errors.validation'));
        return;
      }
    }
    const payload: DeployAppRequest = {
      name: name.trim(),
      template_id: template.id,
      segment,
      internal_port: internalPort,
      subdomain: subdomain.trim() === '' ? undefined : subdomain.trim(),
      expose_externally: expose,
      env: Object.fromEntries(
        Object.entries(envValues).filter(([, value]) => value !== ''),
      ),
    };
    setBusy(true);
    try {
      await appsApi.deploy(payload);
      toast.push({ level: 'success', message: t('apps.toast.deployed', { name: name.trim() }) });
      onDeployed();
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Drawer
      open
      onClose={busy ? () => undefined : onClose}
      width="lg"
      title={t('apps.deploy.title', { name: template.name })}
      subtitle={t('apps.deploy.subtitle')}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Plus size={16} />}
            {busy ? t('apps.deploying') : t('apps.deploy')}
          </button>
        </>
      }
    >
      <div className="space-y-5">
        {error !== null ? (
          <p role="alert" className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300">
            {error}
          </p>
        ) : null}

        <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
          <p className="text-[13px] font-medium text-ink-100">{template.name}</p>
          <p className="mt-1 text-[12px] leading-relaxed text-ink-400">{template.description}</p>
          <p className="mt-2 truncate font-mono text-[11px] text-ink-500">{template.image}</p>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('apps.field.name')} htmlFor="deploy-name" required>
            <input
              id="deploy-name"
              className="input"
              value={name}
              onChange={(event) => setName(event.target.value)}
              autoCapitalize="none"
              spellCheck={false}
            />
          </Field>
          <Field label={t('apps.field.port')} htmlFor="deploy-port" required>
            <input
              id="deploy-port"
              className="input tabular"
              type="number"
              min="1"
              max="65535"
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
          </Field>
        </div>

        <Field label={t('apps.field.segment')}>
          <Segmented
            value={segment}
            onChange={setSegment}
            options={[
              { value: 'lan', label: t('apps.segmentLan') },
              { value: 'dmz', label: t('apps.segmentDmz') },
            ]}
          />
        </Field>
        {segment === 'dmz' ? (
          <p className="-mt-2 rounded-xl border border-amber-400/20 bg-amber-400/[0.06] px-3.5 py-2.5 text-[12px] leading-relaxed text-amber-200">
            {t('apps.segmentWarning')}
          </p>
        ) : null}

        <Field
          label={t('apps.field.subdomain')}
          htmlFor="deploy-subdomain"
          hint={t('apps.field.subdomainHint', { example: `${subdomain.trim() === '' ? template.id : subdomain.trim()}.home.local` })}
        >
          <input
            id="deploy-subdomain"
            className="input input-mono"
            value={subdomain}
            onChange={(event) => setSubdomain(event.target.value)}
            autoCapitalize="none"
            spellCheck={false}
          />
        </Field>

        <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
          <Toggle
            checked={expose}
            onChange={setExpose}
            label={t('apps.field.expose')}
            description={t('apps.segmentWarning')}
          />
        </div>

        <div className="space-y-3">
          <p className="field-label">{t('apps.field.env')}</p>
          {template.env.length === 0 ? (
            <p className="text-xs text-ink-500">{t('common.none')}</p>
          ) : (
            template.env.map((variable) => (
              <EnvField
                key={variable.key}
                variable={variable}
                value={envValues[variable.key] ?? ''}
                onChange={(value) =>
                  setEnvValues((current) => ({ ...current, [variable.key]: value }))
                }
              />
            ))
          )}
        </div>
      </div>
    </Drawer>
  );
}

function EnvField({
  variable,
  value,
  onChange,
}: {
  variable: AppTemplateEnv;
  value: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const marks: string[] = [];
  if (variable.required) marks.push(t('apps.requiredMark'));
  if (variable.secret) marks.push(t('apps.secretMark'));
  if (variable.default !== null && variable.default !== '') {
    marks.push(t('apps.defaultMark', { value: variable.default }));
  }
  return (
    <Field
      label={
        <span>
          {variable.label} <span className="font-mono text-[11px] text-ink-500">{variable.key}</span>
        </span>
      }
      htmlFor={`env-${variable.key}`}
      hint={marks.join(' · ')}
      required={variable.required}
    >
      <input
        id={`env-${variable.key}`}
        className="input"
        type={variable.secret ? 'password' : 'text'}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        autoComplete="off"
        spellCheck={false}
      />
    </Field>
  );
}
