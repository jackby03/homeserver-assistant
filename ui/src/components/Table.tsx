import type { ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { SkeletonRows } from './Skeleton';

export interface Column<T> {
  key: string;
  header: ReactNode;
  render: (row: T, index: number) => ReactNode;
  className?: string;
  headerClassName?: string;
  align?: 'left' | 'right' | 'center';
}

interface DataTableProps<T> {
  columns: Column<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  onRowClick?: (row: T) => void;
  empty?: ReactNode;
  loading?: boolean;
  skeletonRows?: number;
  dense?: boolean;
  caption?: string;
  className?: string;
}

const alignClasses = {
  left: 'text-left',
  right: 'text-right',
  center: 'text-center',
} as const;

/** Dense, accessible data table with a responsive horizontal scroll shell. */
export function DataTable<T>({
  columns,
  rows,
  rowKey,
  onRowClick,
  empty,
  loading = false,
  skeletonRows = 4,
  dense = false,
  caption,
  className,
}: DataTableProps<T>): React.JSX.Element {
  if (loading && rows.length === 0) {
    return (
      <div className="overflow-hidden rounded-2xl border border-white/[0.06]">
        <SkeletonRows rows={skeletonRows} columns={Math.min(columns.length, 6)} />
      </div>
    );
  }

  if (rows.length === 0) {
    return <div>{empty}</div>;
  }

  return (
    <div className={cn('overflow-x-auto rounded-2xl border border-white/[0.06] bg-white/[0.012]', className)}>
      <table className="w-full min-w-[640px] border-collapse text-sm">
        {caption !== undefined ? <caption className="sr-only">{caption}</caption> : null}
        <thead>
          <tr className="border-b border-white/[0.06]">
            {columns.map((column) => (
              <th
                key={column.key}
                scope="col"
                className={cn(
                  'whitespace-nowrap px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.12em] text-ink-500',
                  alignClasses[column.align ?? 'left'],
                  column.headerClassName,
                )}
              >
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => {
            const key = rowKey(row);
            return (
              <tr
                key={key}
                onClick={onRowClick !== undefined ? () => onRowClick(row) : undefined}
                tabIndex={onRowClick !== undefined ? 0 : undefined}
                onKeyDown={
                  onRowClick !== undefined
                    ? (event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault();
                          onRowClick(row);
                        }
                      }
                    : undefined
                }
                className={cn(
                  'border-b border-white/[0.04] last:border-b-0 transition-colors duration-150',
                  onRowClick !== undefined && 'cursor-pointer hover:bg-white/[0.035]',
                )}
              >
                {columns.map((column) => (
                  <td
                    key={column.key}
                    className={cn(
                      'px-4 text-ink-200',
                      dense ? 'py-2.5' : 'py-3.5',
                      alignClasses[column.align ?? 'left'],
                      column.className,
                    )}
                  >
                    {column.render(row, index)}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
