import { useEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import {
  ChevronDown,
  Command,
  Languages,
  LogOut,
  Menu,
  MonitorPlay,
  Search,
} from 'lucide-react';
import { StatusPill } from '@/components/StatusPill';
import { useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { formatDate, formatTime } from '@/lib/format';
import { useNow } from '@/lib/useInterval';
import { useAuth } from '@/state/AuthProvider';

const PAGE_TITLES: Record<string, DictKey> = {
  dashboard: 'nav.dashboard',
  instances: 'nav.instances',
  apps: 'nav.apps',
  homeassistant: 'nav.homeAssistant',
  network: 'nav.network',
  agent: 'nav.agent',
  profiles: 'nav.profiles',
  audit: 'nav.audit',
};

interface TopbarProps {
  onOpenMenu: () => void;
  onOpenPalette: () => void;
}

export function Topbar({ onOpenMenu, onOpenPalette }: TopbarProps): React.JSX.Element {
  const { t, language, setLanguage } = useI18n();
  const { user, logout } = useAuth();
  const location = useLocation();
  const navigate = useNavigate();
  const now = useNow(1_000);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

  const segment = location.pathname.split('/')[2] ?? 'dashboard';
  const titleKey = PAGE_TITLES[segment];

  useEffect(() => {
    setMenuOpen(false);
  }, [location.pathname]);

  useEffect(() => {
    if (!menuOpen) return undefined;
    const onClick = (event: MouseEvent): void => {
      if (menuRef.current !== null && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') setMenuOpen(false);
    };
    document.addEventListener('mousedown', onClick);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onClick);
      document.removeEventListener('keydown', onKey);
    };
  }, [menuOpen]);

  return (
    <header className="sticky top-0 z-30 border-b border-white/[0.05] bg-night-900/70 backdrop-blur-2xl">
      <div className="flex h-16 items-center gap-3 px-4 sm:px-6">
        <button
          type="button"
          onClick={onOpenMenu}
          className="btn btn-ghost btn-icon lg:hidden"
          aria-label={t('nav.admin')}
        >
          <Menu size={19} />
        </button>

        <div className="min-w-0 flex-1">
          <h1 className="truncate font-display text-[17px] font-medium tracking-tight text-ink-50">
            {titleKey !== undefined ? t(titleKey) : t('app.name')}
          </h1>
          <p className="hidden truncate text-[11px] text-ink-500 sm:block">
            {formatDate(now, language)} · {formatTime(now, language, { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })}
          </p>
        </div>

        <div className="flex items-center gap-1.5">
          <StatusPill
            label={t('common.connected')}
            tone="success"
            pulse
            className="hidden sm:inline-flex"
          />

          <button
            type="button"
            onClick={onOpenPalette}
            className="hidden min-h-[40px] items-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.03] px-3 text-[12px] text-ink-400 transition hover:border-white/15 hover:text-ink-100 md:flex"
            aria-label={t('cmd.title')}
          >
            <Search size={14} />
            <span>{t('common.search')}</span>
            <span className="kbd ml-2 inline-flex items-center gap-0.5">
              <Command size={10} />K
            </span>
          </button>

          <button
            type="button"
            onClick={() => setLanguage(language === 'es' ? 'en' : 'es')}
            className="btn btn-ghost min-h-[40px] gap-1.5 px-2.5 text-[12px] uppercase text-ink-300"
            aria-label={t('common.language')}
            title={t('common.language')}
          >
            <Languages size={15} />
            {language}
          </button>

          <div className="relative" ref={menuRef}>
            <button
              type="button"
              onClick={() => setMenuOpen((open) => !open)}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              className="flex min-h-[44px] items-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 transition hover:border-white/15"
            >
              <span className="grid h-7 w-7 place-items-center rounded-full border border-mint-400/20 bg-mint-400/[0.1] text-[11px] font-semibold text-mint-300">
                {initials(user?.display_name ?? 'HS')}
              </span>
              <span className="hidden text-left sm:block">
                <span className="block max-w-[120px] truncate text-[12px] font-medium text-ink-100">
                  {user?.display_name ?? '—'}
                </span>
                <span className="block text-[10px] uppercase tracking-wider text-ink-500">
                  {user?.role ?? ''}
                </span>
              </span>
              <ChevronDown size={14} className={cn('text-ink-500 transition', menuOpen && 'rotate-180')} />
            </button>

            {menuOpen ? (
              <div
                role="menu"
                className="glass-strong absolute right-0 mt-2 w-56 overflow-hidden rounded-2xl p-1.5 animate-[hs-scale-in_160ms_var(--hs-ease)]"
              >
                <div className="px-3 py-2">
                  <p className="truncate text-[13px] font-medium text-ink-100">
                    {user?.display_name ?? '—'}
                  </p>
                  <p className="truncate text-[11px] text-ink-500">{user?.username ?? ''}</p>
                </div>
                <div className="hairline my-1" />
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => navigate('/kiosk')}
                  className="flex min-h-[40px] w-full items-center gap-2.5 rounded-lg px-3 text-[13px] text-ink-200 transition hover:bg-white/[0.06]"
                >
                  <MonitorPlay size={15} className="text-ink-500" />
                  {t('nav.kiosk')}
                </button>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    void logout();
                  }}
                  className="flex min-h-[40px] w-full items-center gap-2.5 rounded-lg px-3 text-[13px] text-ink-200 transition hover:bg-coral-500/10 hover:text-coral-300"
                >
                  <LogOut size={15} className="text-ink-500" />
                  {t('common.signOut')}
                </button>
              </div>
            ) : null}
          </div>
        </div>
      </div>
    </header>
  );
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).slice(0, 2);
  return parts.map((part) => part.charAt(0).toUpperCase()).join('') || 'HS';
}
