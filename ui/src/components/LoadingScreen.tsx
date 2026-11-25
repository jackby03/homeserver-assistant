import { BrandMark } from './BrandMark';
import { Spinner } from './Spinner';

interface LoadingScreenProps {
  message: string;
  hint?: string;
}

/** Full-viewport boot screen shared by the SPA shell and route guards. */
export function LoadingScreen({ message, hint }: LoadingScreenProps): React.JSX.Element {
  return (
    <div className="relative grid min-h-screen place-items-center overflow-hidden px-6">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -top-40 left-1/2 h-[520px] w-[820px] -translate-x-1/2 rounded-full bg-mint-400/[0.06] blur-3xl"
      />
      <div className="animate-fade-up flex flex-col items-center gap-7 text-center">
        <BrandMark size={54} />
        <div className="flex items-center gap-3 text-sm text-ink-300">
          <Spinner />
          <span>{message}</span>
        </div>
        {hint !== undefined ? <p className="max-w-sm text-xs leading-relaxed text-ink-500">{hint}</p> : null}
        <div className="h-px w-40 overflow-hidden rounded-full bg-white/[0.06]">
          <div className="h-full w-1/3 animate-[hs-scan-line_1.8s_ease-in-out_infinite] bg-mint-gradient" />
        </div>
      </div>
    </div>
  );
}
