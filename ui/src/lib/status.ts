import type { DictKey } from '@/i18n/dictionary';

export type Tone = 'success' | 'warning' | 'danger' | 'neutral' | 'info' | 'accent';

/** Localized label key for a runtime/app/audit status. */
export function statusKey(status: string): DictKey | null {
  switch (status) {
    case 'running':
      return 'status.running';
    case 'stopped':
      return 'status.stopped';
    case 'starting':
      return 'status.starting';
    case 'stopping':
      return 'status.stopping';
    case 'frozen':
      return 'status.frozen';
    case 'error':
      return 'status.error';
    case 'unknown':
      return 'status.unknown';
    case 'pending':
      return 'status.pending';
    case 'deploying':
      return 'status.deploying';
    case 'failed':
      return 'status.failed';
    case 'removed':
      return 'status.removed';
    case 'success':
      return 'status.success';
    case 'denied':
      return 'status.denied';
    case 'failure':
      return 'status.failure';
    case 'active':
      return 'status.active';
    case 'inactive':
      return 'status.inactive';
    default:
      return null;
  }
}

export function statusTone(status: string): Tone {
  switch (status) {
    case 'running':
    case 'success':
    case 'active':
      return 'success';
    case 'starting':
    case 'stopping':
    case 'pending':
    case 'deploying':
    case 'frozen':
      return 'warning';
    case 'error':
    case 'failed':
    case 'denied':
    case 'failure':
      return 'danger';
    default:
      return 'neutral';
  }
}
