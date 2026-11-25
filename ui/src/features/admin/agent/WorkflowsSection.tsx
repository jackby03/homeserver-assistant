import { useMemo, useState } from 'react';
import {
  ArrowDown,
  ArrowUp,
  Clock,
  GitBranch,
  Pencil,
  Play,
  Plus,
  Trash2,
  Workflow as WorkflowIcon,
  Wrench,
  Zap,
} from 'lucide-react';
import { agentApi } from '@/api/agent';
import type {
  Profile,
  ToolDefinition,
  UpsertWorkflowRequest,
  Workflow,
  WorkflowNode,
  WorkflowRunResponse,
  WorkflowTrigger,
} from '@/api/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { EmptyState } from '@/components/EmptyState';
import { Field, Segmented, Toggle } from '@/components/Field';
import { Modal } from '@/components/Modal';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { formatRelative } from '@/lib/format';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';

type NodeKind = 'trigger' | 'tool' | 'condition' | 'delay';

interface DraftNode {
  id: string;
  kind: NodeKind;
  triggerType: WorkflowTrigger['type'];
  webhookPath: string;
  tool: string;
  argumentsText: string;
  field: string;
  equalsText: string;
  seconds: number;
}

let draftId = 1;
const nextDraftId = (): string => {
  const id = `node-${draftId}`;
  draftId += 1;
  return id;
};

function toDraft(node: WorkflowNode): DraftNode {
  const base: DraftNode = {
    id: node.id,
    kind: node.type,
    triggerType: 'manual',
    webhookPath: '',
    tool: '',
    argumentsText: '',
    field: '',
    equalsText: '""',
    seconds: 1,
  };
  if (node.type === 'trigger') {
    return {
      ...base,
      triggerType: node.trigger.type,
      webhookPath: node.trigger.type === 'webhook' ? node.trigger.path : '',
    };
  }
  if (node.type === 'tool') {
    return { ...base, tool: node.tool, argumentsText: JSON.stringify(node.arguments ?? {}, null, 2) };
  }
  if (node.type === 'condition') {
    return { ...base, field: node.field, equalsText: JSON.stringify(node.equals ?? null) };
  }
  return { ...base, seconds: node.seconds };
}

function draftToNode(node: DraftNode): WorkflowNode {
  if (node.kind === 'trigger') {
    const trigger: WorkflowTrigger =
      node.triggerType === 'webhook'
        ? { type: 'webhook', path: node.webhookPath }
        : { type: node.triggerType };
    return { id: node.id, type: 'trigger', trigger };
  }
  if (node.kind === 'tool') {
    const parsed: unknown = node.argumentsText.trim() === '' ? {} : JSON.parse(node.argumentsText);
    return { id: node.id, type: 'tool', tool: node.tool, arguments: parsed };
  }
  if (node.kind === 'condition') {
    const parsed: unknown = node.equalsText.trim() === '' ? null : JSON.parse(node.equalsText);
    return { id: node.id, type: 'condition', field: node.field, equals: parsed };
  }
  return { id: node.id, type: 'delay', seconds: Math.max(1, Math.round(node.seconds)) };
}

function defaultNodes(): DraftNode[] {
  return [
    {
      id: nextDraftId(),
      kind: 'trigger',
      triggerType: 'manual',
      webhookPath: '',
      tool: '',
      argumentsText: '',
      field: '',
      equalsText: '""',
      seconds: 1,
    },
    {
      id: nextDraftId(),
      kind: 'tool',
      triggerType: 'manual',
      webhookPath: '',
      tool: '',
      argumentsText: '{\n  "text": "hello"\n}',
      field: '',
      equalsText: '""',
      seconds: 1,
    },
  ];
}

interface WorkflowsSectionProps {
  profiles: Profile[];
  tools: ToolDefinition[];
  /** Open the editor immediately (command palette deep link). */
  startWithEditor?: boolean;
}

export function WorkflowsSection({
  profiles,
  tools,
  startWithEditor = false,
}: WorkflowsSectionProps): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const [refreshToken, setRefreshToken] = useState(0);
  const workflows = useResource(() => agentApi.workflows(), [refreshToken]);
  const [editor, setEditor] = useState<{ workflow: Workflow | null } | null>(
    startWithEditor ? { workflow: null } : null,
  );
  const [runTarget, setRunTarget] = useState<Workflow | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Workflow | null>(null);
  const [deleting, setDeleting] = useState(false);

  const refresh = (): void => setRefreshToken((value) => value + 1);
  const list = workflows.data?.workflows ?? [];

  const removeWorkflow = async (): Promise<void> => {
    if (deleteTarget === null) return;
    setDeleting(true);
    try {
      await agentApi.deleteWorkflow(deleteTarget.id);
      toast.push({ level: 'success', message: t('agent.toast.workflowDeleted') });
      setDeleteTarget(null);
      refresh();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setDeleting(false);
    }
  };

  return (
    <section className="glass space-y-4 p-5 sm:p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="grid h-8 w-8 place-items-center rounded-xl border border-mint-400/20 bg-mint-400/[0.07] text-mint-300">
            <WorkflowIcon size={16} />
          </span>
          <div>
            <h2 className="font-display text-[15px] font-medium text-ink-100">{t('agent.workflows.title')}</h2>
            <p className="text-[11px] text-ink-500">{t('agent.workflows.subtitle')}</p>
          </div>
        </div>
        <button type="button" className="btn btn-primary btn-sm" onClick={() => setEditor({ workflow: null })}>
          <Plus size={15} />
          {t('agent.workflows.new')}
        </button>
      </div>

      {workflows.loading && list.length === 0 ? (
        <div className="space-y-2">
          <div className="skeleton h-20 w-full rounded-xl" />
          <div className="skeleton h-20 w-full rounded-xl" />
        </div>
      ) : list.length === 0 ? (
        <EmptyState
          icon={<WorkflowIcon size={20} />}
          title={t('agent.workflows.empty')}
          action={
            <button type="button" className="btn btn-secondary btn-sm" onClick={() => setEditor({ workflow: null })}>
              <Plus size={15} />
              {t('agent.workflows.new')}
            </button>
          }
        />
      ) : (
        <ul className="space-y-2">
          {list.map((workflow) => {
            const trigger = workflow.nodes.find((node) => node.type === 'trigger');
            const triggerType = trigger?.type === 'trigger' ? trigger.trigger.type : 'manual';
            return (
              <li
                key={workflow.id}
                className="flex flex-col gap-3 rounded-2xl border border-white/[0.06] bg-white/[0.015] p-3.5 transition hover:border-white/[0.12] sm:flex-row sm:items-center sm:justify-between"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="truncate text-[13px] font-medium text-ink-100">{workflow.name}</p>
                    <StatusPill
                      label={workflow.enabled ? t('common.enabled') : t('common.disabled')}
                      tone={workflow.enabled ? 'success' : 'neutral'}
                    />
                    <span className="chip">
                      <Zap size={11} />
                      {t(`agent.workflows.trigger.${triggerType}` as 'agent.workflows.trigger.manual')}
                    </span>
                  </div>
                  <p className="truncate text-[11.5px] text-ink-500">
                    {workflow.description !== '' ? workflow.description : t('common.none')} ·{' '}
                    {t('agent.workflows.nodes')}: {workflow.nodes.length} ·{' '}
                    {formatRelative(workflow.updated_at, language)}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={() => setRunTarget(workflow)}
                    disabled={!workflow.enabled}
                  >
                    <Play size={13} />
                    {t('agent.workflows.run')}
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-icon"
                    onClick={() => setEditor({ workflow })}
                    aria-label={t('common.edit')}
                  >
                    <Pencil size={15} />
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-icon text-ink-500 hover:bg-coral-500/10 hover:text-coral-300"
                    onClick={() => setDeleteTarget(workflow)}
                    aria-label={t('agent.workflows.delete')}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {editor !== null ? (
        <WorkflowEditor
          workflow={editor.workflow}
          tools={tools}
          onClose={() => setEditor(null)}
          onSaved={() => {
            setEditor(null);
            refresh();
          }}
        />
      ) : null}

      {runTarget !== null ? (
        <WorkflowRunDialog
          workflow={runTarget}
          profiles={profiles}
          onClose={() => setRunTarget(null)}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => void removeWorkflow()}
        busy={deleting}
        title={t('agent.workflows.confirmDelete.title', { name: deleteTarget?.name ?? '' })}
        message={t('agent.workflows.confirmDelete.body')}
        confirmLabel={t('common.delete')}
      />
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Editor                                                                     */
/* -------------------------------------------------------------------------- */

function WorkflowEditor({
  workflow,
  tools,
  onClose,
  onSaved,
}: {
  workflow: Workflow | null;
  tools: ToolDefinition[];
  onClose: () => void;
  onSaved: () => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState(workflow?.name ?? '');
  const [description, setDescription] = useState(workflow?.description ?? '');
  const [enabled, setEnabled] = useState(workflow?.enabled ?? true);
  const [nodes, setNodes] = useState<DraftNode[]>(() =>
    workflow !== null ? workflow.nodes.map(toDraft) : defaultNodes(),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const updateNode = (id: string, patch: Partial<DraftNode>): void => {
    setNodes((current) => current.map((node) => (node.id === id ? { ...node, ...patch } : node)));
  };

  const move = (index: number, direction: -1 | 1): void => {
    setNodes((current) => {
      const next = [...current];
      const target = index + direction;
      const item = next[index];
      const swapped = next[target];
      if (item === undefined || swapped === undefined) return current;
      next[index] = swapped;
      next[target] = item;
      return next;
    });
  };

  const addNode = (kind: Exclude<NodeKind, 'trigger'>): void => {
    setNodes((current) => [
      ...current,
      {
        id: nextDraftId(),
        kind,
        triggerType: 'manual',
        webhookPath: '',
        tool: tools[0]?.name ?? '',
        argumentsText: kind === 'tool' ? '{}' : '',
        field: '',
        equalsText: '""',
        seconds: 1,
      },
    ]);
  };

  const save = async (): Promise<void> => {
    setError(null);
    if (name.trim() === '') {
      setError(t('errors.validation'));
      return;
    }
    let parsed: WorkflowNode[];
    try {
      parsed = nodes.map(draftToNode);
    } catch {
      setError(t('agent.workflows.invalidJson'));
      return;
    }
    const edges = parsed.slice(1).map((node, index) => ({
      from: parsed[index]?.id ?? '',
      to: node.id,
    }));
    const payload: UpsertWorkflowRequest = {
      name: name.trim(),
      description: description.trim(),
      enabled,
      nodes: parsed,
      edges,
    };
    setBusy(true);
    try {
      if (workflow === null) {
        await agentApi.createWorkflow(payload);
      } else {
        await agentApi.updateWorkflow(workflow.id, payload);
      }
      toast.push({ level: 'success', message: t('agent.toast.workflowSaved') });
      onSaved();
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="xl"
      title={workflow === null ? t('agent.workflows.new') : t('common.edit')}
      description={t('agent.workflows.subtitle')}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void save()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <WorkflowIcon size={16} />}
            {busy ? t('agent.workflows.saving') : t('agent.workflows.save')}
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

        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('agent.workflows.name')} htmlFor="workflow-name" required>
            <input
              id="workflow-name"
              className="input"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={t('agent.workflows.namePlaceholder')}
            />
          </Field>
          <Field label={t('agent.workflows.description')} htmlFor="workflow-description">
            <input
              id="workflow-description"
              className="input"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </Field>
        </div>

        <Toggle
          checked={enabled}
          onChange={setEnabled}
          label={t('common.enabled')}
          description={t('agent.workflows.subtitle')}
        />

        <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="field-label mb-0">{t('agent.workflows.nodes')}</p>
            <div className="flex flex-wrap gap-1.5">
              <button type="button" className="btn btn-secondary btn-sm" onClick={() => addNode('tool')}>
                <Wrench size={13} />
                {t('agent.workflows.node.tool')}
              </button>
              <button type="button" className="btn btn-secondary btn-sm" onClick={() => addNode('condition')}>
                <GitBranch size={13} />
                {t('agent.workflows.node.condition')}
              </button>
              <button type="button" className="btn btn-secondary btn-sm" onClick={() => addNode('delay')}>
                <Clock size={13} />
                {t('agent.workflows.node.delay')}
              </button>
            </div>
          </div>

          <ol className="space-y-2">
            {nodes.map((node, index) => (
              <li
                key={node.id}
                className={cn(
                  'rounded-2xl border p-3.5',
                  node.kind === 'trigger'
                    ? 'border-mint-400/20 bg-mint-400/[0.03]'
                    : 'border-white/[0.07] bg-white/[0.015]',
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span className="grid h-6 w-6 place-items-center rounded-lg border border-white/[0.08] bg-white/[0.03] font-mono text-[10px] text-ink-400">
                      {index + 1}
                    </span>
                    <span className="text-[11px] font-medium uppercase tracking-wider text-ink-400">
                      {t(`agent.workflows.node.${node.kind}` as 'agent.workflows.node.tool')}
                    </span>
                  </div>
                  <div className="flex items-center gap-0.5">
                    <button
                      type="button"
                      className="grid h-8 w-8 place-items-center rounded-lg text-ink-500 transition hover:bg-white/[0.06] hover:text-ink-200 disabled:opacity-30"
                      onClick={() => move(index, -1)}
                      disabled={index === 0}
                      aria-label={t('common.back')}
                    >
                      <ArrowUp size={14} />
                    </button>
                    <button
                      type="button"
                      className="grid h-8 w-8 place-items-center rounded-lg text-ink-500 transition hover:bg-white/[0.06] hover:text-ink-200 disabled:opacity-30"
                      onClick={() => move(index, 1)}
                      disabled={index === nodes.length - 1}
                      aria-label={t('common.next')}
                    >
                      <ArrowDown size={14} />
                    </button>
                    <button
                      type="button"
                      className="grid h-8 w-8 place-items-center rounded-lg text-ink-500 transition hover:bg-coral-500/10 hover:text-coral-300 disabled:opacity-30"
                      onClick={() => setNodes((current) => current.filter((item) => item.id !== node.id))}
                      disabled={node.kind === 'trigger' && index === 0}
                      aria-label={t('agent.workflows.removeNode')}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>

                <div className="mt-3 space-y-3">
                  {node.kind === 'trigger' ? (
                    <>
                      <Field label={t('agent.workflows.triggerKind')}>
                        <Segmented
                          value={node.triggerType}
                          onChange={(triggerType) => updateNode(node.id, { triggerType })}
                          options={[
                            { value: 'manual', label: t('agent.workflows.trigger.manual') },
                            { value: 'profile_activated', label: t('agent.workflows.trigger.profile_activated') },
                            { value: 'webhook', label: t('agent.workflows.trigger.webhook') },
                          ]}
                        />
                      </Field>
                      {node.triggerType === 'webhook' ? (
                        <Field label={t('agent.workflows.webhookPath')} htmlFor={`webhook-${node.id}`}>
                          <input
                            id={`webhook-${node.id}`}
                            className="input input-mono"
                            value={node.webhookPath}
                            onChange={(event) => updateNode(node.id, { webhookPath: event.target.value })}
                            placeholder="/hooks/good-night"
                            spellCheck={false}
                          />
                        </Field>
                      ) : null}
                    </>
                  ) : null}

                  {node.kind === 'tool' ? (
                    <>
                      <Field label={t('agent.workflows.node.tool')} htmlFor={`tool-${node.id}`}>
                        <select
                          id={`tool-${node.id}`}
                          className="input"
                          value={node.tool}
                          onChange={(event) => updateNode(node.id, { tool: event.target.value })}
                        >
                          <option value="">{t('agent.workflows.node.toolPlaceholder')}</option>
                          {tools.map((tool) => (
                            <option key={tool.name} value={tool.name}>
                              {tool.name}
                            </option>
                          ))}
                        </select>
                      </Field>
                      <Field label={t('agent.workflows.node.arguments')} htmlFor={`args-${node.id}`}>
                        <textarea
                          id={`args-${node.id}`}
                          className="input input-mono min-h-[84px] resize-y"
                          value={node.argumentsText}
                          onChange={(event) => updateNode(node.id, { argumentsText: event.target.value })}
                          spellCheck={false}
                        />
                      </Field>
                    </>
                  ) : null}

                  {node.kind === 'condition' ? (
                    <div className="grid gap-3 sm:grid-cols-2">
                      <Field label={t('agent.workflows.node.field')} htmlFor={`field-${node.id}`}>
                        <input
                          id={`field-${node.id}`}
                          className="input input-mono"
                          value={node.field}
                          onChange={(event) => updateNode(node.id, { field: event.target.value })}
                          placeholder="output.state"
                          spellCheck={false}
                        />
                      </Field>
                      <Field label={t('agent.workflows.node.equals')} htmlFor={`equals-${node.id}`}>
                        <input
                          id={`equals-${node.id}`}
                          className="input input-mono"
                          value={node.equalsText}
                          onChange={(event) => updateNode(node.id, { equalsText: event.target.value })}
                          placeholder='"on"'
                          spellCheck={false}
                        />
                      </Field>
                    </div>
                  ) : null}

                  {node.kind === 'delay' ? (
                    <Field label={t('agent.workflows.node.seconds')} htmlFor={`seconds-${node.id}`}>
                      <input
                        id={`seconds-${node.id}`}
                        className="input tabular"
                        type="number"
                        min="1"
                        max="3600"
                        value={node.seconds}
                        onChange={(event) => updateNode(node.id, { seconds: Number(event.target.value) })}
                      />
                    </Field>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>
        </div>
      </div>
    </Modal>
  );
}

/* -------------------------------------------------------------------------- */
/* Run dialog                                                                 */
/* -------------------------------------------------------------------------- */

function WorkflowRunDialog({
  workflow,
  profiles,
  onClose,
}: {
  workflow: Workflow;
  profiles: Profile[];
  onClose: () => void;
}): React.JSX.Element {
  const { t } = useI18n();
  const [profileId, setProfileId] = useState(profiles[0]?.id ?? '');
  const [inputText, setInputText] = useState('{}');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<WorkflowRunResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const stepsText = useMemo(
    () => (result !== null ? JSON.stringify(result.steps, null, 2) : ''),
    [result],
  );
  const outputText = useMemo(
    () => (result !== null ? JSON.stringify(result.output, null, 2) : ''),
    [result],
  );

  const run = async (): Promise<void> => {
    setError(null);
    if (profileId === '') {
      setError(t('agent.workflows.noProfiles'));
      return;
    }
    let input: unknown = inputText;
    if (inputText.trim() !== '') {
      try {
        input = JSON.parse(inputText);
      } catch {
        setError(t('agent.workflows.invalidJson'));
        return;
      }
    }
    setBusy(true);
    try {
      const response = await agentApi.runWorkflow(workflow.id, { profile_id: profileId, input });
      setResult(response);
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="lg"
      title={t('agent.workflows.run')}
      description={workflow.name}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.close')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void run()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Play size={16} />}
            {busy ? t('agent.workflows.running') : t('agent.workflows.run')}
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

        <Field label={t('agent.workflows.selectProfile')} htmlFor="run-profile">
          <select
            id="run-profile"
            className="input"
            value={profileId}
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

        <Field label={t('agent.workflows.inputJson')} htmlFor="run-input">
          <textarea
            id="run-input"
            className="input input-mono min-h-[84px] resize-y"
            value={inputText}
            onChange={(event) => setInputText(event.target.value)}
            spellCheck={false}
          />
        </Field>

        {result !== null ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <StatusPill
                label={result.success ? t('agent.workflows.runSuccess') : t('agent.workflows.runFailed')}
                tone={result.success ? 'success' : 'danger'}
              />
            </div>
            <div>
              <p className="field-label">{t('agent.workflows.steps')}</p>
              <pre className="max-h-52 overflow-auto rounded-xl border border-white/[0.06] bg-[#040810] p-3 font-mono text-[10.5px] leading-relaxed text-ink-300">
                {stepsText}
              </pre>
            </div>
            <div>
              <p className="field-label">{t('agent.workflows.output')}</p>
              <pre className="max-h-40 overflow-auto rounded-xl border border-white/[0.06] bg-[#040810] p-3 font-mono text-[10.5px] leading-relaxed text-ink-300">
                {outputText}
              </pre>
            </div>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
