import { useId } from 'react';
import { cn } from '@/lib/cn';

type SparkTone = 'mint' | 'amber' | 'coral';

const strokeTones: Record<SparkTone, string> = {
  mint: '#6ee7ff',
  amber: '#fbbf24',
  coral: '#fb7185',
};

interface SparklineProps {
  values: number[];
  className?: string;
  tone?: SparkTone;
  /** Accessible description; when omitted the chart is decorative. */
  ariaLabel?: string;
  height?: number;
}

const VIEW_WIDTH = 120;
const VIEW_HEIGHT = 36;
const PADDING = 2;

/** Compact SVG sparkline fed by a ring buffer of samples. */
export function Sparkline({
  values,
  className,
  tone = 'mint',
  ariaLabel,
  height = 36,
}: SparklineProps): React.JSX.Element {
  const rawId = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const gradientId = `spark-${rawId}`;
  const samples = values.filter((value) => Number.isFinite(value));
  const stroke = strokeTones[tone];

  let line = '';
  let area = '';
  if (samples.length >= 2) {
    const min = Math.min(...samples);
    const max = Math.max(...samples);
    const span = max - min < 0.001 ? 1 : max - min;
    const innerWidth = VIEW_WIDTH - PADDING * 2;
    const innerHeight = VIEW_HEIGHT - PADDING * 2;
    const points = samples.map((value, index) => {
      const x = PADDING + (index / (samples.length - 1)) * innerWidth;
      const y = PADDING + (1 - (value - min) / span) * innerHeight;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    });
    line = `M${points.join(' L')}`;
    area = `${line} L${(PADDING + innerWidth).toFixed(2)},${VIEW_HEIGHT - PADDING} L${PADDING},${
      VIEW_HEIGHT - PADDING
    } Z`;
  }

  return (
    <svg
      viewBox={`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`}
      preserveAspectRatio="none"
      className={cn('w-full', className)}
      style={{ height }}
      role={ariaLabel !== undefined ? 'img' : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel === undefined ? true : undefined}
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={stroke} stopOpacity="0.28" />
          <stop offset="100%" stopColor={stroke} stopOpacity="0" />
        </linearGradient>
      </defs>
      {samples.length >= 2 ? (
        <>
          <path d={area} fill={`url(#${gradientId})`} />
          <path
            d={line}
            fill="none"
            stroke={stroke}
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
          <circle
            cx={VIEW_WIDTH - PADDING}
            cy={
              PADDING +
              (1 -
                (samples[samples.length - 1]! - Math.min(...samples)) /
                  (Math.max(...samples) - Math.min(...samples) < 0.001
                    ? 1
                    : Math.max(...samples) - Math.min(...samples))) *
                (VIEW_HEIGHT - PADDING * 2)
            }
            r="2"
            fill={stroke}
          />
        </>
      ) : (
        <line
          x1={PADDING}
          y1={VIEW_HEIGHT / 2}
          x2={VIEW_WIDTH - PADDING}
          y2={VIEW_HEIGHT / 2}
          stroke={stroke}
          strokeOpacity="0.25"
          strokeWidth="1.4"
          strokeDasharray="3 4"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}
