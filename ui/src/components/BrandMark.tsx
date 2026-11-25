import { cn } from '@/lib/cn';

interface BrandMarkProps {
  size?: number;
  withWordmark?: boolean;
  className?: string;
}

/**
 * Product mark: a layered shield/hexagon with a live core node.
 * Rendered as inline SVG so it stays crisp on the kiosk wall display.
 */
export function BrandMark({ size = 34, withWordmark = false, className }: BrandMarkProps): React.JSX.Element {
  return (
    <span className={cn('inline-flex items-center gap-3', className)}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 40 40"
        fill="none"
        aria-hidden="true"
        className="shrink-0"
      >
        <defs>
          <linearGradient id="hs-brand-gradient" x1="6" y1="4" x2="34" y2="36" gradientUnits="userSpaceOnUse">
            <stop stopColor="#6ee7ff" />
            <stop offset="1" stopColor="#34d399" />
          </linearGradient>
          <linearGradient id="hs-brand-inner" x1="12" y1="10" x2="28" y2="30" gradientUnits="userSpaceOnUse">
            <stop stopColor="#ffffff" stopOpacity="0.9" />
            <stop offset="1" stopColor="#6ee7ff" stopOpacity="0.35" />
          </linearGradient>
        </defs>
        <path
          d="M20 3.5 34.5 12v16L20 36.5 5.5 28V12L20 3.5Z"
          stroke="url(#hs-brand-gradient)"
          strokeWidth="2"
          strokeLinejoin="round"
          fill="rgba(110,231,255,0.06)"
        />
        <path
          d="M20 10.5 28.5 15.5v9L20 29.5 11.5 24.5v-9L20 10.5Z"
          stroke="url(#hs-brand-inner)"
          strokeWidth="1.4"
          strokeLinejoin="round"
          fill="none"
        />
        <circle cx="20" cy="20" r="3.2" fill="url(#hs-brand-gradient)" />
        <circle cx="20" cy="20" r="6.4" stroke="#6ee7ff" strokeOpacity="0.28" strokeWidth="1" />
      </svg>
      {withWordmark ? (
        <span className="font-display text-[17px] font-medium leading-none tracking-tight text-ink-50">
          HomeServer
          <span className="block text-[10px] font-normal uppercase tracking-[0.24em] text-mint-400/80">
            Assistant OS
          </span>
        </span>
      ) : null}
    </span>
  );
}
