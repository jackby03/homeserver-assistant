import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router-dom';
import {
  Boxes,
  Bot,
  House,
  LayoutDashboard,
  LayoutGrid,
  LogOut,
  MonitorPlay,
  Network,
  RefreshCw,
  ScrollText,
  Search,
  UsersRound,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { useAuth } from '@/state/AuthProvider';

interface PaletteItem {
  id: string;
  labelKey: DictKey;
  icon: LucideIcon;
  group: 'navigation' | 'actions';
  run: (navigate: ReturnType<typeof useNavigate>) => void;
}

const ITEMS: PaletteItem[] = [
  {
    id: 'nav-dashboard',
    labelKey: 'nav.dashboard',
    icon: LayoutDashboard,
    group: 'navigation',
    run: (navigate) => navigate('/admin/dashboard'),
  },
  {
    id: 'nav-instances',
    labelKey: 'nav.instances',
    icon: Boxes,
    group: 'navigation',
    run: (navigate) => navigate('/admin/instances'),
  },
  {
    id: 'nav-apps',
    labelKey: 'nav.apps',
    icon: LayoutGrid,
    group: 'navigation',
    run: (navigate) => navigate('/admin/apps'),
  },
  {
    id: 'nav-homeassistant',
    labelKey: 'nav.homeAssistant',
    icon: House,
    group: 'navigation',
    run: (navigate) => navigate('/admin/homeassistant'),
  },
  {
    id: 'nav-network',
    labelKey: 'nav.network',
    icon: Network,
    group: 'navigation',
    run: (navigate) => navigate('/admin/network'),
  },
  {
    id: 'nav-agent',
    labelKey: 'nav.agent',
    icon: Bot,
    group: 'navigation',
    run: (navigate) => navigate('/admin/agent'),
  },
  {
    id: 'nav-profiles',
    labelKey: 'nav.profiles',
    icon: UsersRound,
    group: 'navigation',
    run: (navigate) => navigate('/admin/profiles'),
  },
  {
    id: 'nav-audit',
    labelKey: 'nav.audit',
    icon: ScrollText,
    group: 'navigation',
    run: (navigate) => navigate('/admin/audit'),
  },
  {
    id: 'action-new-instance',
    labelKey: 'cmd.newInstance',
    icon: Boxes,
    group: 'actions',
    run: (navigate) => navigate('/admin/instances?new=1'),
  },
  {
    id: 'action-new-profile',
    labelKey: 'cmd.newProfile',
    icon: UsersRound,
    group: 'actions',
    run: (navigate) => navigate('/admin/profiles?new=1'),
  },
  {
    id: 'action-new-provider',
    labelKey: 'cmd.newProvider',
    icon: Bot,
    group: 'actions',
    run: (navigate) => navigate('/admin/agent?addProvider=1'),
  },
  {
    id: 'action-new-workflow',
    labelKey: 'cmd.newWorkflow',
    icon: ScrollText,
    group: 'actions',
    run: (navigate) => navigate('/admin/agent?newWorkflow=1'),
  },
  {
    id: 'action-refresh',
    labelKey: 'cmd.refresh',
    icon: RefreshCw,
    group: 'actions',
    run: () => window.location.reload(),
  },
  {
    id: 'action-kiosk',
    labelKey: 'cmd.openKiosk',
    icon: MonitorPlay,
    group: 'actions',
    run: () => window.open('/kiosk', '_blank', 'noopener'),
  },
];

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

/** Cmd/Ctrl+K navigation and quick actions. */
export function CommandPalette({ open, onClose }: CommandPaletteProps): React.JSX.Element | null {
  const { t } = useI18n();
  const navigate = useNavigate();
  const { logout } = useAuth();
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const items = useMemo(() => {
    const all: PaletteItem[] = [
      ...ITEMS,
      {
        id: 'action-logout',
        labelKey: 'cmd.logout',
        icon: LogOut,
        group: 'actions',
        run: () => {
          void logout();
        },
      },
    ];
    const normalized = query.trim().toLocaleLowerCase();
    if (normalized === '') return all;
    return all.filter((item) => t(item.labelKey).toLocaleLowerCase().includes(normalized));
  }, [query, t, logout]);

  useEffect(() => {
    if (!open) {
      setQuery('');
      setActiveIndex(0);
      return;
    }
    setActiveIndex(0);
    const timer = window.setTimeout(() => inputRef.current?.focus(), 20);
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') onClose();
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setActiveIndex((current) => Math.min(items.length - 1, current + 1));
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setActiveIndex((current) => Math.max(0, current - 1));
      }
      if (event.key === 'Enter') {
        const item = items[activeIndex];
        if (item !== undefined) {
          event.preventDefault();
          item.run(navigate);
          onClose();
        }
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.clearTimeout(timer);
    };
  }, [open, items, activeIndex, onClose, navigate]);

  if (!open) return null;

  const groups: Array<{ key: 'navigation' | 'actions'; labelKey: DictKey }> = [
    { key: 'navigation', labelKey: 'cmd.navigation' },
    { key: 'actions', labelKey: 'cmd.actions' },
  ];

  return createPortal(
    <div className="fixed inset-0 z-[70] flex items-start justify-center px-4 pt-[12vh]">
      <div
        aria-hidden="true"
        className="absolute inset-0 animate-[hs-fade-in_150ms_ease] bg-night-950/70 backdrop-blur-sm"
        onMouseDown={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t('cmd.title')}
        className="glass-strong relative z-10 w-full max-w-xl overflow-hidden rounded-[22px] animate-[hs-scale-in_180ms_var(--hs-ease)]"
      >
        <div className="flex items-center gap-3 border-b border-white/[0.07] px-4 py-3.5">
          <Search size={17} className="text-ink-500" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActiveIndex(0);
            }}
            placeholder={t('cmd.placeholder')}
            aria-label={t('cmd.placeholder')}
            className="min-h-[32px] flex-1 border-none bg-transparent text-[14px] text-ink-100 placeholder:text-ink-500 focus:outline-none"
          />
          <span className="kbd hidden sm:inline-flex">Esc</span>
        </div>
        <div className="max-h-[52vh] overflow-y-auto p-2">
          {items.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-ink-500">{t('cmd.noResults')}</p>
          ) : (
            groups.map((group) => {
              const groupItems = items.filter((item) => item.group === group.key);
              if (groupItems.length === 0) return null;
              return (
                <div key={group.key} className="mb-1">
                  <p className="px-3 py-1.5">{t(group.labelKey)}</p>
                  {groupItems.map((item) => {
                    const globalIndex = items.indexOf(item);
                    const active = globalIndex === activeIndex;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onMouseEnter={() => setActiveIndex(globalIndex)}
                        onClick={() => {
                          item.run(navigate);
                          onClose();
                        }}
                        className={cn(
                          'flex min-h-[42px] w-full items-center gap-3 rounded-xl px-3 text-left text-[13px] transition',
                          active
                            ? 'bg-mint-400/[0.09] text-ink-50'
                            : 'text-ink-300 hover:bg-white/[0.04] hover:text-ink-100',
                        )}
                      >
                        <item.icon
                          size={16}
                          className={active ? 'text-mint-400' : 'text-ink-500'}
                        />
                        <span className="flex-1 truncate">{t(item.labelKey)}</span>
                        {active ? <span className="text-[11px] text-mint-400/80">↵</span> : null}
                      </button>
                    );
                  })}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
