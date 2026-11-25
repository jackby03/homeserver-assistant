import { useId } from 'react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

/* -------------------------------------------------------------------------- */
/* Field                                                                      */
/* -------------------------------------------------------------------------- */

interface FieldProps {
  label: ReactNode;
  htmlFor?: string;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  optionalLabel?: string;
  className?: string;
  children: ReactNode;
}

export function Field({
  label,
  htmlFor,
  hint,
  error,
  required = false,
  optionalLabel,
  className,
  children,
}: FieldProps): React.JSX.Element {
  return (
    <div className={cn('min-w-0', className)}>
      <label className="field-label flex items-baseline gap-1.5" htmlFor={htmlFor}>
        <span>{label}</span>
        {required ? (
          <span className="text-coral-300" aria-hidden="true">
            *
          </span>
        ) : null}
        {optionalLabel !== undefined ? (
          <span className="text-[11px] font-normal text-ink-500">{optionalLabel}</span>
        ) : null}
      </label>
      {children}
      {error !== undefined && error !== null && error !== '' ? (
        <p className="field-error" role="alert">
          {error}
        </p>
      ) : hint !== undefined ? (
        <p className="field-hint">{hint}</p>
      ) : null}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Toggle                                                                     */
/* -------------------------------------------------------------------------- */

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  className?: string;
}

export function Toggle({
  checked,
  onChange,
  label,
  description,
  disabled = false,
  className,
}: ToggleProps): React.JSX.Element {
  const id = useId();
  return (
    <div className={cn('flex items-start justify-between gap-4', className)}>
      <div className="min-w-0">
        <label htmlFor={id} className="cursor-pointer text-sm font-medium text-ink-100">
          {label}
        </label>
        {description !== undefined ? (
          <p className="mt-1 text-xs leading-relaxed text-ink-400">{description}</p>
        ) : null}
      </div>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cn(
          'relative mt-0.5 h-6 w-11 shrink-0 rounded-full border transition duration-200 ease-smooth focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-mint-400/70 focus-visible:ring-offset-2 focus-visible:ring-offset-night-900 disabled:opacity-40',
          checked
            ? 'border-mint-400/40 bg-mint-500/80'
            : 'border-white/10 bg-white/[0.07] hover:bg-white/[0.11]',
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            'absolute top-1/2 h-4 w-4 -translate-y-1/2 rounded-full bg-white shadow-sm transition-all duration-200 ease-smooth',
            checked ? 'left-[calc(100%-1.125rem)]' : 'left-1',
          )}
        />
      </button>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Segmented control                                                          */
/* -------------------------------------------------------------------------- */

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
  icon?: ReactNode;
  disabled?: boolean;
}

interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  label?: string;
  className?: string;
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
  className,
}: SegmentedProps<T>): React.JSX.Element {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn(
        'inline-flex min-h-[44px] w-full items-center gap-1 rounded-xl border border-white/[0.08] bg-night-800/60 p-1',
        className,
      )}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={option.disabled}
            onClick={() => onChange(option.value)}
            className={cn(
              'inline-flex min-h-[36px] flex-1 items-center justify-center gap-1.5 rounded-lg px-3 text-[13px] font-medium transition duration-150 ease-smooth disabled:opacity-40',
              selected
                ? 'bg-white/[0.09] text-ink-50 shadow-sm'
                : 'text-ink-400 hover:bg-white/[0.04] hover:text-ink-200',
            )}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Checkbox                                                                   */
/* -------------------------------------------------------------------------- */

interface CheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  disabled?: boolean;
  className?: string;
}

export function Checkbox({
  checked,
  onChange,
  label,
  disabled = false,
  className,
}: CheckboxProps): React.JSX.Element {
  const id = useId();
  return (
    <label
      htmlFor={id}
      className={cn(
        'inline-flex min-h-[36px] cursor-pointer items-center gap-2.5 text-[13px] text-ink-200',
        disabled && 'cursor-not-allowed opacity-50',
        className,
      )}
    >
      <span
        className={cn(
          'grid h-[18px] w-[18px] shrink-0 place-items-center rounded-md border transition duration-150 ease-smooth',
          checked
            ? 'border-mint-400/60 bg-mint-500/25 text-mint-200'
            : 'border-white/15 bg-white/[0.04] text-transparent',
        )}
      >
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path
            d="M2 6.2 4.7 9 10 3.4"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </span>
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="sr-only"
      />
      {label}
    </label>
  );
}
