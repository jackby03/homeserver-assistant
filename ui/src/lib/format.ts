/**
 * Display formatting helpers. All user-visible numbers go through these so the
 * UI stays consistent and uses tabular numerals where needed.
 */

export type UiLanguage = 'es' | 'en';

const localeFor = (language: UiLanguage): string => (language === 'es' ? 'es-PE' : 'en-US');

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function formatNumber(value: number, language: UiLanguage, digits = 0): string {
  if (!Number.isFinite(value)) return '—';
  return new Intl.NumberFormat(localeFor(language), {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
}

export function formatPercent(value: number, language: UiLanguage, digits = 0): string {
  return `${formatNumber(clamp(value, 0, 100), language, digits)}%`;
}

/** Format a byte count with binary units, e.g. `1.4 GB`. */
export function formatBytes(bytes: number, language: UiLanguage, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const precision = unit === 0 ? 0 : digits;
  return `${formatNumber(value, language, precision)} ${units[unit]}`;
}

/** Format a per-second byte rate, e.g. `2.1 MB/s`. */
export function formatRate(bytesPerSecond: number, language: UiLanguage): string {
  return `${formatBytes(bytesPerSecond, language, 1)}/s`;
}

/** Format a duration in seconds as a compact human string. */
export function formatDuration(seconds: number, language: UiLanguage): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const total = Math.floor(seconds);
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  const secs = total % 60;
  const unit = (n: number, singular: string, plural: string): string =>
    `${formatNumber(n, language)} ${n === 1 ? singular : plural}`;
  if (days > 0) return `${unit(days, 'd', 'd')} ${unit(hours, 'h', 'h')}`;
  if (hours > 0) return `${unit(hours, 'h', 'h')} ${unit(minutes, 'min', 'min')}`;
  if (minutes > 0) return `${unit(minutes, 'min', 'min')}`;
  return `${formatNumber(secs, language)} s`;
}

export function formatDateTime(
  iso: string | null | undefined,
  language: UiLanguage,
  options?: Intl.DateTimeFormatOptions,
): string {
  if (!iso) return '—';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat(
    localeFor(language),
    options ?? {
      dateStyle: 'medium',
      timeStyle: 'short',
    },
  ).format(date);
}

export function formatTime(
  date: Date,
  language: UiLanguage,
  options?: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat(
    localeFor(language),
    options ?? { hour: '2-digit', minute: '2-digit', hour12: false },
  ).format(date);
}

export function formatDate(date: Date, language: UiLanguage): string {
  return new Intl.DateTimeFormat(localeFor(language), {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
  }).format(date);
}

export function formatRelative(
  iso: string | null | undefined,
  language: UiLanguage,
  now: Date = new Date(),
): string {
  if (!iso) return '—';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '—';
  const deltaSeconds = Math.round((date.getTime() - now.getTime()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(localeFor(language), { numeric: 'auto' });
  const absolute = Math.abs(deltaSeconds);
  if (absolute < 60) return formatter.format(deltaSeconds, 'second');
  if (absolute < 3_600) return formatter.format(Math.round(deltaSeconds / 60), 'minute');
  if (absolute < 86_400) return formatter.format(Math.round(deltaSeconds / 3_600), 'hour');
  return formatter.format(Math.round(deltaSeconds / 86_400), 'day');
}

/** Keep percentages inside [0, 100] even if the backend reports odd numbers. */
export function toPercent(used: number, total: number): number {
  if (!Number.isFinite(used) || !Number.isFinite(total) || total <= 0) return 0;
  return clamp((used / total) * 100, 0, 100);
}

export function truncate(text: string, max: number): string {
  if (text.length <= max) return text;
  return `${text.slice(0, Math.max(0, max - 1))}…`;
}
