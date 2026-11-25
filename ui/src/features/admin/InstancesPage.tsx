import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useSearchParams } from 'react-router-dom';
import {
  Boxes,
  Camera,
  Container,
  FileText,
  Info,
  Monitor,
  Play,
  Plus,
  RotateCcw,
  Square,
  TerminalSquare,
  Trash2,
} from 'lucide-react';
import { instancesApi } from '@/api/instances';
import { isApiError } from '@/api/client';
import type {
  CreateInstanceRequest,
  InstanceSummary,
  NetworkSegment,
  RuntimeKind,
  SnapshotSummary,
} from '@/api/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { DataTable } from '@/components/Table';
import type { Column } from '@/components/Table';
import { Drawer } from '@/components/Drawer';
import { EmptyState } from '@/components/EmptyState';
import { Field, Segmented } from '@/components/Field';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { Terminal } from '@/components/Terminal';
import { Modal } from '@/components/Modal';
import { useI18n } from '@/i18n/I18nProvider';
import { errorMessageKey } from '@/i18n/I18nProvider';
import { formatBytes, formatDateTime, formatNumber } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

type PendingAction = `${string}:${'start' | 'stop' | 'restart'}`;

export function InstancesPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [params] = useSearchParams();
  const [refreshToken, setRefreshToken] = useState(0);
  const [createOpen, setCreateOpen] = useState(params.get('new') === '1');
  const [detailName, setDetailName] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<InstanceSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [pending, setPending] = useState<PendingAction | null>(null);

  const list = useResource(() => instancesApi.list(), [refreshToken], { pollMs: 5_000 });

  const refresh = (): void => setRefreshToken((value) => value + 1);

  const runAction = async (
    name: string,
    action: 'start' | 'stop' | 'restart',
  ): Promise<void> => {
    setPending(`${name}:${action}`);
    try {
      if (action === 'start') await instancesApi.start(name);
      else if (action === 'stop') await instancesApi.stop(name);
      else await instancesApi.restart(name);
      const toastKey =
        action === 'start' ? 'inst.toast.started' : action === 'stop' ? 'inst.toast.stopped' : 'inst.toast.restarted';
      toast.push({ level: 'success', message: t(toastKey, { name }) });
      list.reload();
    } catch (cause) {
      toast.push({
        level: 'error',
        title: t('toast.error'),
        message: t(errorMessageKey(cause)),
      });
    } finally {
      setPending(null);
    }
  };

  const confirmDelete = async (): Promise<void> => {
    if (deleteTarget === null) return;
    setDeleting(true);
    try {
      await instancesApi.remove(deleteTarget.name);
      toast.push({ level: 'success', message: t('inst.toast.deleted', { name: deleteTarget.name }) });
      setDeleteTarget(null);
      list.reload();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setDeleting(false);
    }
  };

  const instances = list.data?.instances ?? [];
  const provider = list.data?.provider ?? null;

  const columns: Column<InstanceSummary>[] = [
    {
      key: 'name',
      header: t('inst.table.name'),
      render: (instance) => (
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-white/[0.07] bg-white/[0.03] text-ink-300">
            {instance.kind === 'vm' ? <Monitor size={15} /> : <Container size={15} />}
          </span>
          <div className="min-w-0">
            <p className="truncate font-mono text-[12.5px] font-medium text-ink-100">{instance.name}</p>
            <p className="text-[11px] text-ink-500">
              {t(instance.kind === 'vm' ? 'inst.kind.vm' : 'inst.kind.container')}
            </p>
          </div>
        </div>
      ),
    },
    {
      key: 'status',
      header: t('inst.table.status'),
      render: (instance) => <StatusPill status={instance.status} pulse={instance.status === 'running'} />,
    },
    {
      key: 'image',
      header: t('inst.table.image'),
      render: (instance) => (
        <span className="block max-w-[190px] truncate font-mono text-[11.5px] text-ink-400" title={instance.image}>
          {instance.image}
        </span>
      ),
    },
    {
      key: 'addresses',
      header: t('inst.table.addresses'),
      render: (instance) =>
        instance.ipv4_addresses.length > 0 ? (
          <span className="font-mono text-[11.5px] text-ink-300">{instance.ipv4_addresses.join(', ')}</span>
        ) : (
          <span className="text-ink-600">—</span>
        ),
    },
    {
      key: 'limits',
      header: t('inst.table.limits'),
      render: (instance) => (
        <span className="text-[11.5px] tabular text-ink-400">
          {instance.limits.cpu_limit !== null ? `${formatNumber(instance.limits.cpu_limit, language, 1)} vCPU` : '—'}
          {' · '}
          {instance.limits.memory_mb !== null ? formatBytes(instance.limits.memory_mb * 1024 * 1024, language) : '—'}
        </span>
      ),
    },
    {
      key: 'created',
      header: t('inst.table.created'),
      render: (instance) => (
        <span className="whitespace-nowrap text-[11.5px] text-ink-500">
          {instance.created_at !== null ? formatDateTime(instance.created_at, language, { dateStyle: 'short', timeStyle: 'short' }) : '—'}
        </span>
      ),
    },
    {
      key: 'actions',
      header: t('inst.table.actions'),
      align: 'right',
      render: (instance) => {
        const running = instance.status === 'running';
        const busy = pending !== null && pending.startsWith(`${instance.name}:`);
        return (
          <div className="flex items-center justify-end gap-1" onClick={(event) => event.stopPropagation()}>
            <IconAction
              label={running ? t('common.stop') : t('common.start')}
              onClick={() => void runAction(instance.name, running ? 'stop' : 'start')}
              disabled={busy || instance.status === 'starting' || instance.status === 'stopping'}
            >
              {busy ? <Spinner size={14} /> : running ? <Square size={14} /> : <Play size={14} />}
            </IconAction>
            <IconAction
              label={t('common.restart')}
              onClick={() => void runAction(instance.name, 'restart')}
              disabled={busy}
            >
              <RotateCcw size={14} />
            </IconAction>
            <IconAction label={t('inst.action.details')} onClick={() => setDetailName(instance.name)}>
              <Info size={14} />
            </IconAction>
            <IconAction
              label={t('inst.action.delete')}
              danger
              onClick={() => setDeleteTarget(instance)}
            >
              <Trash2 size={14} />
            </IconAction>
          </div>
        );
      },
    },
  ];

  return (
    <div className="space-y-5">
      <PageHeader
        title={t('inst.title')}
        subtitle={t('inst.subtitle')}
        actions={
          <>
            <button type="button" className="btn btn-secondary btn-sm" onClick={refresh} disabled={list.refreshing}>
              <RotateCcw size={14} className={list.refreshing ? 'animate-spin' : undefined} />
              {t('common.refresh')}
            </button>
            <button type="button" className="btn btn-primary btn-sm" onClick={() => setCreateOpen(true)}>
              <Plus size={15} />
              {t('inst.new')}
            </button>
          </>
        }
      />

      {provider !== null && !provider.available ? (
        <div className="rounded-2xl border border-amber-400/25 bg-amber-400/[0.07] px-4 py-3 text-sm text-amber-200">
          {t('inst.providerUnavailable', { name: provider.name })}
        </div>
      ) : null}

      {list.error !== null && instances.length === 0 ? (
        <div className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3 text-sm text-coral-200">
          {t(errorMessageKey(list.error))}
        </div>
      ) : null}

      <DataTable
        columns={columns}
        rows={instances}
        rowKey={(instance) => instance.name}
        loading={list.loading}
        onRowClick={(instance) => setDetailName(instance.name)}
        empty={
          <EmptyState
            icon={<Boxes size={20} />}
            title={t('inst.empty.title')}
            body={t('inst.empty.body')}
            action={
              <button type="button" className="btn btn-primary btn-sm" onClick={() => setCreateOpen(true)}>
                <Plus size={15} />
                {t('inst.new')}
              </button>
            }
          />
        }
      />

      <InstanceCreateDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={() => {
          setCreateOpen(false);
          refresh();
        }}
      />

      {detailName !== null ? (
        <InstanceDetailDrawer
          name={detailName}
          onClose={() => setDetailName(null)}
          onChanged={refresh}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => void confirmDelete()}
        busy={deleting}
        title={t('inst.confirmDelete.title', { name: deleteTarget?.name ?? '' })}
        message={t('inst.confirmDelete.body')}
        confirmLabel={t('common.delete')}
      />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Create dialog                                                              */
/* -------------------------------------------------------------------------- */

interface EnvRow {
  id: number;
  key: string;
  value: string;
}

let envRowId = 1;

function InstanceCreateDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState('');
  const [kind, setKind] = useState<RuntimeKind>('container');
  const [image, setImage] = useState('images:debian/13');
  const [segment, setSegment] = useState<'none' | NetworkSegment>('lan');
  const [cpu, setCpu] = useState('');
  const [memory, setMemory] = useState('');
  const [envRows, setEnvRows] = useState<EnvRow[]>([]);
  const [start, setStart] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setName('');
    setKind('container');
    setImage('images:debian/13');
    setSegment('lan');
    setCpu('');
    setMemory('');
    setEnvRows([]);
    setStart(true);
    setError(null);
  }, [open]);

  const submit = async (): Promise<void> => {
    setError(null);
    if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(name)) {
      setError(t('inst.validation.name'));
      return;
    }
    if (image.trim() === '') {
      setError(t('inst.validation.image'));
      return;
    }
    const env: Record<string, string> = {};
    for (const row of envRows) {
      if (row.key.trim() !== '') env[row.key.trim()] = row.value;
    }
    const payload: CreateInstanceRequest = {
      name,
      kind,
      image: image.trim(),
      segment: segment === 'none' ? null : segment,
      limits: {
        cpu_limit: cpu.trim() === '' ? null : Number(cpu),
        memory_mb: memory.trim() === '' ? null : Number(memory),
      },
      env,
      start,
    };
    setBusy(true);
    try {
      await instancesApi.create(payload);
      toast.push({ level: 'success', message: t('inst.toast.created', { name }) });
      onCreated();
    } catch (cause) {
      if (isApiError(cause) && cause.code === 'validation_error') {
        setError(t('errors.validation'));
      } else {
        setError(t(errorMessageKey(cause)));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open={open}
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="lg"
      title={t('inst.create.title')}
      description={t('inst.create.subtitle')}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Plus size={16} />}
            {busy ? t('common.creating') : t('common.create')}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        {error !== null ? (
          <p role="alert" className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300">
            {error}
          </p>
        ) : null}
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('inst.field.name')} htmlFor="instance-name" required hint={t('inst.nameHint')}>
            <input
              id="instance-name"
              className="input"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={t('inst.namePlaceholder')}
              autoCapitalize="none"
              spellCheck={false}
            />
          </Field>
          <Field label={t('inst.field.kind')}>
            <Segmented
              value={kind}
              onChange={setKind}
              options={[
                { value: 'container', label: t('inst.kind.container'), icon: <Container size={14} /> },
                { value: 'vm', label: t('inst.kind.vm'), icon: <Monitor size={14} /> },
              ]}
            />
          </Field>
        </div>
        <Field label={t('inst.field.image')} htmlFor="instance-image" required>
          <input
            id="instance-image"
            className="input input-mono"
            value={image}
            onChange={(event) => setImage(event.target.value)}
            placeholder={t('inst.imagePlaceholder')}
            spellCheck={false}
          />
        </Field>
        <Field label={t('inst.field.segment')}>
          <Segmented
            value={segment}
            onChange={setSegment}
            options={[
              { value: 'lan', label: t('apps.segmentLan') },
              { value: 'dmz', label: t('apps.segmentDmz') },
              { value: 'none', label: t('inst.field.segmentNone') },
            ]}
          />
        </Field>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('inst.field.cpu')} htmlFor="instance-cpu" hint={t('inst.limits.cpuHint')}>
            <input
              id="instance-cpu"
              className="input tabular"
              type="number"
              min="0.1"
              step="0.1"
              value={cpu}
              onChange={(event) => setCpu(event.target.value)}
              placeholder="1.0"
            />
          </Field>
          <Field label={t('inst.field.memory')} htmlFor="instance-memory" hint={t('inst.limits.memoryHint')}>
            <input
              id="instance-memory"
              className="input tabular"
              type="number"
              min="64"
              step="64"
              value={memory}
              onChange={(event) => setMemory(event.target.value)}
              placeholder="1024"
            />
          </Field>
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <p className="field-label mb-0">{t('inst.field.env')}</p>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setEnvRows((rows) => [...rows, { id: envRowId++, key: '', value: '' }])}
            >
              <Plus size={14} />
              {t('common.addRow')}
            </button>
          </div>
          {envRows.length === 0 ? (
            <p className="text-xs text-ink-500">{t('common.none')}</p>
          ) : (
            <div className="space-y-2">
              {envRows.map((row) => (
                <div key={row.id} className="flex items-center gap-2">
                  <input
                    className="input input-mono flex-1"
                    value={row.key}
                    placeholder={t('inst.field.envKey')}
                    onChange={(event) =>
                      setEnvRows((rows) => rows.map((item) => (item.id === row.id ? { ...item, key: event.target.value } : item)))
                    }
                    aria-label={t('inst.field.envKey')}
                  />
                  <input
                    className="input flex-1"
                    value={row.value}
                    placeholder={t('inst.field.envValue')}
                    onChange={(event) =>
                      setEnvRows((rows) => rows.map((item) => (item.id === row.id ? { ...item, value: event.target.value } : item)))
                    }
                    aria-label={t('inst.field.envValue')}
                  />
                  <button
                    type="button"
                    className="btn btn-ghost btn-icon text-ink-500 hover:text-coral-300"
                    onClick={() => setEnvRows((rows) => rows.filter((item) => item.id !== row.id))}
                    aria-label={t('common.remove')}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <label className="flex min-h-[44px] cursor-pointer items-center gap-2.5 rounded-xl border border-white/[0.06] bg-white/[0.02] px-3 text-[13px] text-ink-200">
          <input
            type="checkbox"
            checked={start}
            onChange={(event) => setStart(event.target.checked)}
            className="h-4 w-4 accent-[#34d399]"
          />
          {t('inst.field.startNow')}
        </label>
      </div>
    </Modal>
  );
}

/* -------------------------------------------------------------------------- */
/* Detail drawer                                                              */
/* -------------------------------------------------------------------------- */

type DetailTab = 'overview' | 'snapshots' | 'logs' | 'terminal';

function InstanceDetailDrawer({
  name,
  onClose,
  onChanged,
}: {
  name: string;
  onClose: () => void;
  onChanged: () => void;
}): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [tab, setTab] = useState<DetailTab>('overview');
  const [refreshToken, setRefreshToken] = useState(0);
  const [snapshotName, setSnapshotName] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ kind: 'restore' | 'delete'; snapshot: string } | null>(null);

  const instance = useResource(() => instancesApi.get(name), [name, refreshToken]);
  const snapshots = useResource(() => instancesApi.snapshots(name), [name, refreshToken]);
  const logs = useResource(() => instancesApi.logs(name, 200), [name, refreshToken]);

  const [cpu, setCpu] = useState('');
  const [memory, setMemory] = useState('');

  useEffect(() => {
    const limits = instance.data?.limits;
    if (limits === undefined) return;
    setCpu(limits.cpu_limit !== null ? String(limits.cpu_limit) : '');
    setMemory(limits.memory_mb !== null ? String(limits.memory_mb) : '');
  }, [instance.data]);

  const saveLimits = async (): Promise<void> => {
    setBusy('limits');
    try {
      await instancesApi.updateLimits(name, {
        cpu_limit: cpu.trim() === '' ? null : Number(cpu),
        memory_mb: memory.trim() === '' ? null : Number(memory),
      });
      toast.push({ level: 'success', message: t('inst.limits.saved') });
      setRefreshToken((value) => value + 1);
      onChanged();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(null);
    }
  };

  const createSnapshot = async (): Promise<void> => {
    if (snapshotName.trim() === '') return;
    setBusy('snapshot');
    try {
      await instancesApi.createSnapshot(name, { name: snapshotName.trim() });
      toast.push({ level: 'success', message: t('inst.snapshots.toastCreated') });
      setSnapshotName('');
      setRefreshToken((value) => value + 1);
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(null);
    }
  };

  const runSnapshotAction = async (): Promise<void> => {
    if (confirm === null) return;
    setBusy(confirm.kind);
    try {
      if (confirm.kind === 'restore') {
        await instancesApi.restoreSnapshot(name, confirm.snapshot);
        toast.push({ level: 'success', message: t('inst.snapshots.toastRestored') });
      } else {
        await instancesApi.deleteSnapshot(name, confirm.snapshot);
        toast.push({ level: 'success', message: t('inst.snapshots.toastDeleted') });
      }
      setConfirm(null);
      setRefreshToken((value) => value + 1);
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(null);
    }
  };

  const tabs: Array<{ key: DetailTab; label: string; icon: ReactNode }> = [
    { key: 'overview', label: t('inst.detail.overview'), icon: <Info size={14} /> },
    { key: 'snapshots', label: t('inst.detail.snapshots'), icon: <Camera size={14} /> },
    { key: 'logs', label: t('inst.detail.logs'), icon: <FileText size={14} /> },
    { key: 'terminal', label: t('inst.detail.terminal'), icon: <TerminalSquare size={14} /> },
  ];

  return (
    <Drawer
      open
      onClose={onClose}
      width="xl"
      title={t('inst.detail.title', { name })}
      subtitle={
        instance.data !== null
          ? t(instance.data.kind === 'vm' ? 'inst.kind.vm' : 'inst.kind.container')
          : undefined
      }
    >
      <div className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          {instance.data !== null ? <StatusPill status={instance.data.status} pulse={instance.data.status === 'running'} /> : null}
          {instance.data !== null ? (
            <span className="chip font-mono">{instance.data.image}</span>
          ) : null}
        </div>

        <div className="flex gap-1 overflow-x-auto rounded-xl border border-white/[0.06] bg-night-800/50 p-1">
          {tabs.map((item) => (
            <button
              key={item.key}
              type="button"
              onClick={() => setTab(item.key)}
              className={
                tab === item.key
                  ? 'inline-flex min-h-[36px] flex-1 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg bg-white/[0.08] px-3 text-[12.5px] font-medium text-ink-50'
                  : 'inline-flex min-h-[36px] flex-1 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg px-3 text-[12.5px] text-ink-400 transition hover:text-ink-100'
              }
            >
              {item.icon}
              {item.label}
            </button>
          ))}
        </div>

        {tab === 'overview' ? (
          <div className="space-y-4">
            <dl className="grid gap-x-6 gap-y-3 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4 sm:grid-cols-2">
              <Detail label={t('inst.table.addresses')}>
                {instance.data?.ipv4_addresses.join(', ') || '—'}
              </Detail>
              <Detail label={t('inst.table.snapshots')}>
                {instance.data?.snapshot_count !== undefined ? String(instance.data.snapshot_count) : '—'}
              </Detail>
              <Detail label={t('inst.table.created')}>
                {instance.data?.created_at != null ? formatDateTime(instance.data.created_at, language) : '—'}
              </Detail>
              <Detail label={t('inst.field.segment')}>
                {instance.data !== null
                  ? t(instance.data.kind === 'vm' ? 'inst.kind.vm' : 'inst.kind.container')
                  : '—'}
              </Detail>
            </dl>

            <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] p-4">
              <p className="mb-3 text-[13px] font-medium text-ink-100">{t('inst.limits.title')}</p>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={t('inst.field.cpu')} htmlFor="limits-cpu" hint={t('inst.limits.cpuHint')}>
                  <input
                    id="limits-cpu"
                    className="input tabular"
                    type="number"
                    step="0.1"
                    min="0.1"
                    value={cpu}
                    onChange={(event) => setCpu(event.target.value)}
                  />
                </Field>
                <Field label={t('inst.field.memory')} htmlFor="limits-memory" hint={t('inst.limits.memoryHint')}>
                  <input
                    id="limits-memory"
                    className="input tabular"
                    type="number"
                    step="64"
                    min="64"
                    value={memory}
                    onChange={(event) => setMemory(event.target.value)}
                  />
                </Field>
              </div>
              <button
                type="button"
                className="btn btn-secondary btn-sm mt-3"
                onClick={() => void saveLimits()}
                disabled={busy !== null}
              >
                {busy === 'limits' ? <Spinner size={14} /> : null}
                {t('common.save')}
              </button>
            </div>
          </div>
        ) : null}

        {tab === 'snapshots' ? (
          <div className="space-y-3">
            <div className="flex items-end gap-2">
              <Field label={t('inst.snapshots.name')} htmlFor="snapshot-name" className="flex-1">
                <input
                  id="snapshot-name"
                  className="input"
                  value={snapshotName}
                  onChange={(event) => setSnapshotName(event.target.value)}
                  placeholder={t('inst.snapshots.namePlaceholder')}
                />
              </Field>
              <button
                type="button"
                className="btn btn-primary btn-sm mb-0.5"
                onClick={() => void createSnapshot()}
                disabled={busy !== null || snapshotName.trim() === ''}
              >
                {busy === 'snapshot' ? <Spinner size={14} className="text-[#04121a]" /> : <Plus size={15} />}
                {t('inst.snapshots.new')}
              </button>
            </div>
            {snapshots.loading && snapshots.data === null ? (
              <div className="skeleton h-16 w-full" />
            ) : snapshots.data === null || snapshots.data.snapshots.length === 0 ? (
              <EmptyState title={t('inst.snapshots.empty')} />
            ) : (
              <ul className="divide-y divide-white/[0.05] rounded-2xl border border-white/[0.06]">
                {snapshots.data.snapshots.map((snapshot: SnapshotSummary) => (
                  <li key={snapshot.name} className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                    <div className="min-w-0">
                      <p className="truncate font-mono text-[12.5px] text-ink-100">{snapshot.name}</p>
                      <p className="text-[11px] text-ink-500">
                        {snapshot.created_at !== null ? formatDateTime(snapshot.created_at, language) : '—'}
                        {' · '}
                        {t('inst.snapshots.size')}: {formatBytes(snapshot.size_bytes, language)}
                      </p>
                    </div>
                    <div className="flex gap-1.5">
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => setConfirm({ kind: 'restore', snapshot: snapshot.name })}
                      >
                        <RotateCcw size={13} />
                        {t('inst.snapshots.restore')}
                      </button>
                      <button
                        type="button"
                        className="btn btn-danger btn-sm"
                        onClick={() => setConfirm({ kind: 'delete', snapshot: snapshot.name })}
                        aria-label={t('common.delete')}
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
        ) : null}

        {tab === 'logs' ? (
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <p className="text-xs text-ink-500">{t('inst.logs.lines', { count: 200 })}</p>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={() => setRefreshToken((value) => value + 1)}
              >
                <RotateCcw size={13} className={logs.refreshing ? 'animate-spin' : undefined} />
                {t('inst.logs.refresh')}
              </button>
            </div>
            <div className="max-h-[420px] overflow-auto rounded-2xl border border-white/[0.07] bg-[#040810] p-4">
              {logs.data !== null && logs.data.logs.trim() !== '' ? (
                <pre className="whitespace-pre-wrap break-words font-mono text-[11.5px] leading-relaxed text-ink-300">
                  {logs.data.logs}
                </pre>
              ) : (
                <p className="text-sm text-ink-500">{t('inst.logs.empty')}</p>
              )}
            </div>
          </div>
        ) : null}

        {tab === 'terminal' ? <Terminal instanceName={name} /> : null}
      </div>

      <ConfirmDialog
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        onConfirm={() => void runSnapshotAction()}
        busy={busy === 'restore' || busy === 'delete'}
        title={
          confirm?.kind === 'restore'
            ? `${t('inst.snapshots.restore')} ${confirm.snapshot}`
            : `${t('common.delete')} ${confirm?.snapshot ?? ''}`
        }
        message={t('inst.confirmDelete.body')}
        confirmLabel={confirm?.kind === 'restore' ? t('inst.snapshots.restore') : t('common.delete')}
        danger={confirm?.kind === 'delete'}
      />
    </Drawer>
  );
}

function Detail({ label, children }: { label: string; children: ReactNode }): React.JSX.Element {
  return (
    <div className="min-w-0">
      <dt className="text-[11px] uppercase tracking-wider text-ink-500">{label}</dt>
      <dd className="mt-0.5 truncate font-mono text-[12.5px] text-ink-200">{children}</dd>
    </div>
  );
}

function IconAction({
  label,
  onClick,
  children,
  disabled = false,
  danger = false,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  disabled?: boolean;
  danger?: boolean;
}): React.JSX.Element {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={
        danger
          ? 'grid h-9 min-h-[36px] w-9 min-w-[36px] place-items-center rounded-lg text-ink-400 transition hover:bg-coral-500/10 hover:text-coral-300 disabled:opacity-40'
          : 'grid h-9 min-h-[36px] w-9 min-w-[36px] place-items-center rounded-lg text-ink-400 transition hover:bg-white/[0.07] hover:text-ink-50 disabled:opacity-40'
      }
    >
      {children}
    </button>
  );
}
