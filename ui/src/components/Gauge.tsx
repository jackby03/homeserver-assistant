import { useId } from 'react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';

const RADIUS = 46;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

type GaugeTone = 'mint' | 'amber' | 'coral';

const gradients: Record<GaugeTone, [string, string]> = {
  mint: ['#6ee7ff', '#34d399'],
  amber: ['#fcd34d', '#f59e0b'],
  coral: ['#fda4af', '#fb7185'],
};

interface GaugeProps {
  value: number;
  label: string;
  caption?: ReactNode;
  unit?: string;
  size?: number;
  tone?: GaugeTone | 'auto';
  className?: string;
}

function autoTone(value: number): GaugeTone {
  if (value >= 90) return 'coral';
  if (value >= 70) return 'amber';
  return 'mint';
}

/** Circular metric gauge with a gradient arc and tabular value. */
export function Gauge({
  value,
  label,
  caption,
  unit = '%',
  size = 132,
  tone = 'auto',
  className,
}: GaugeProps): React.JSX.Element {
  const rawId = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const gradientId = `gauge-${rawId}`;
  const clamped = Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  const resolvedTone = tone === 'auto' ? autoTone(clamped) : tone;
  const [from, to] = gradients[resolvedTone];
  const offset = CIRCUMFERENCE * (1 - clamped / 100);

  return (
    <div className={cn('flex flex-col items-center gap-2', className)}>
      <div className="relative" style={{ width: size, height: size }}>
        <svg
          viewBox="0 0 120 120"
          width={size}
          height={size}
          role="img"
          aria-label={`${label}: ${Math.round(clamped)}${unit}`}
        >
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0%" stopColor={from} />
              <stop offset="100%" stopColor={to} />
            </linearGradient>
          </defs>
          <circle cx="60" cy="60" r={RADIUS} fill="none" stroke="rgba(255,255,255,0.06)" strokeWidth="7" />
          <circle
            cx="60"
            cy="60"
            r={RADIUS}
            fill="none"
            stroke={`url(#${gradientId})`}
            strokeWidth="7"
            strokeLinecap="round"
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={offset}
            transform="rotate(-90 60 60)"
            className="transition-[stroke-dashoffset] duration-700 ease-smooth"
          />
          <circle cx="60" cy="60" r="39" fill="rgba(255,255,255,0.015)" />
          <text
            x="60"
            y="57"
            textAnchor="middle"
            fill="#f2f7fb"
            style={{ font: '500 24px "Outfit Variable", sans-serif', fontVariantNumeric: 'tabular-nums' }}
          >
            {Math.round(clamped)}
          </text>
          <text
            x="60"
            y="74"
            textAnchor="middle"
            fill="#8da3b8"
            style={{ font: '500 10px "Inter Variable", sans-serif', letterSpacing: '0.08em' }}
          >
            {unit}
          </text>
        </svg>
      </div>
      <div className="text-center">
        <p className="text-[13px] font-medium text-ink-200">{label}</p>
        {caption !== undefined ? (
          <p className="mt-0.5 text-[11px] leading-relaxed text-ink-500">{caption}</p>
        ) : null}
      </div>
    </div>
  );
}
