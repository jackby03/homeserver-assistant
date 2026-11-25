import { cn } from '@/lib/cn';

interface SkeletonProps {
  className?: string;
}

export function Skeleton({ className }: SkeletonProps): React.JSX.Element {
  return <div className={cn('skeleton h-4 w-full', className)} aria-hidden="true" />;
}

interface SkeletonRowsProps {
  rows?: number;
  columns?: number;
}

export function SkeletonRows({ rows = 4, columns = 4 }: SkeletonRowsProps): React.JSX.Element {
  return (
    <div className="divide-y divide-white/[0.04]">
      {Array.from({ length: rows }).map((_, rowIndex) => (
        <div key={rowIndex} className="flex items-center gap-4 px-4 py-4">
          {Array.from({ length: columns }).map((__, columnIndex) => (
            <Skeleton
              key={columnIndex}
              className={cn('h-3.5', columnIndex === 0 ? 'w-1/4' : 'w-1/6')}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

interface SkeletonCardProps {
  lines?: number;
  className?: string;
}

export function SkeletonCard({ lines = 3, className }: SkeletonCardProps): React.JSX.Element {
  return (
    <div className={cn('glass space-y-3 p-5', className)}>
      <Skeleton className="h-3 w-1/3" />
      <Skeleton className="h-7 w-1/2" />
      {Array.from({ length: Math.max(0, lines - 2) }).map((_, index) => (
        <Skeleton key={index} className="h-3 w-2/3" />
      ))}
    </div>
  );
}
