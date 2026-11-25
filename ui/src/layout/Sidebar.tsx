import { Link, NavLink } from 'react-router-dom';
import {
  Boxes,
  ChevronRight,
  House,
  LayoutDashboard,
  LayoutGrid,
  LogOut,
  MonitorPlay,
  Network,
  ScrollText,
  UsersRound,
  Bot,
  X,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { BrandMark } from '@/components/BrandMark';
import { useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { useAuth } from '@/state/AuthProvider';

interface NavItem {
  to: string;
  labelKey: DictKey;
  icon: LucideIcon;
}

interface NavGroup {
  labelKey: DictKey;
  items: NavItem[];
}

const NAV_GROUPS: NavGroup[] = [
  {
    labelKey: 'nav.group.operate',
    items: [
      { to: '/admin/dashboard', labelKey: 'nav.dashboard', icon: LayoutDashboard },
      { to: '/admin/instances', labelKey: 'nav.instances', icon: Boxes },
      { to: '/admin/apps', labelKey: 'nav.apps', icon: LayoutGrid },
      { to: '/admin/homeassistant', labelKey: 'nav.homeAssistant', icon: House },
    ],
  },
  {
    labelKey: 'nav.group.system',
    items: [
      { to: '/admin/network', labelKey: 'nav.network', icon: Network },
      { to: '/admin/audit', labelKey: 'nav.audit', icon: ScrollText },
    ],
  },
  {
    labelKey: 'nav.group.intelligence',
    items: [
      { to: '/admin/agent', labelKey: 'nav.agent', icon: Bot },
      { to: '/admin/profiles', labelKey: 'nav.profiles', icon: UsersRound },
    ],
  },
];

interface SidebarProps {
  mobileOpen: boolean;
  onCloseMobile: () => void;
  connected: boolean;
  version: string | null;
}

export function Sidebar({
  mobileOpen,
  onCloseMobile,
  connected,
  version,
}: SidebarProps): React.JSX.Element {
  const { t } = useI18n();
  const { user, logout } = useAuth();

  return (
    <>
      <div
        aria-hidden="true"
        onClick={onCloseMobile}
        className={cn(
          'fixed inset-0 z-40 bg-night-950/70 backdrop-blur-sm transition-opacity duration-200 lg:hidden',
          mobileOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      />
      <aside
        className={cn(
          'fixed inset-y-0 left-0 z-40 flex w-[272px] flex-col border-r border-white/[0.06] bg-night-900/85 backdrop-blur-2xl transition-transform duration-200 ease-smooth lg:sticky lg:top-0 lg:z-20 lg:h-screen lg:translate-x-0',
          mobileOpen ? 'translate-x-0' : '-translate-x-full',
        )}
        aria-label={t('nav.admin')}
      >
        <div className="flex items-center justify-between gap-3 px-5 pb-4 pt-5">
          <Link
            to="/admin/dashboard"
            onClick={onCloseMobile}
            className="rounded-xl focus-visible:outline-none"
            aria-label={t('app.name')}
          >
            <BrandMark size={36} withWordmark />
          </Link>
          <button
            type="button"
            onClick={onCloseMobile}
            className="btn btn-ghost btn-icon lg:hidden"
            aria-label={t('common.close')}
          >
            <X size={18} />
          </button>
        </div>

        <nav className="flex-1 space-y-6 overflow-y-auto px-3 pb-4 pt-2">
          {NAV_GROUPS.map((group) => (
            <div key={group.labelKey} className="space-y-1">
              <p className="px-3 pb-1">{t(group.labelKey)}</p>
              {group.items.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  onClick={onCloseMobile}
                  className={({ isActive }) => cn('group nav-link', isActive && 'nav-link-active')}
                >
                  {({ isActive }) => (
                    <>
                      <item.icon
                        size={18}
                        className={cn(
                          'shrink-0 transition-colors',
                          isActive ? 'text-mint-400' : 'text-ink-500 group-hover:text-ink-300',
                        )}
                      />
                      <span className="flex-1 truncate">{t(item.labelKey)}</span>
                      {isActive ? (
                        <ChevronRight size={14} className="text-mint-400/70" aria-hidden="true" />
                      ) : null}
                    </>
                  )}
                </NavLink>
              ))}
            </div>
          ))}
        </nav>

        <div className="border-t border-white/[0.05] px-4 py-4">
          <Link
            to="/kiosk"
            target="_blank"
            rel="noreferrer"
            className="flex min-h-[44px] items-center gap-2.5 rounded-xl px-2.5 text-[13px] text-ink-300 transition hover:bg-white/[0.05] hover:text-ink-50"
          >
            <MonitorPlay size={16} className="text-ink-500" />
            <span className="flex-1">{t('nav.kiosk')}</span>
            <ChevronRight size={13} className="text-ink-600" aria-hidden="true" />
          </Link>
          <div className="mt-3 flex items-center justify-between gap-2 rounded-xl border border-white/[0.05] bg-white/[0.02] px-3 py-2.5">
            <div className="flex min-w-0 items-center gap-2">
              <span
                aria-hidden="true"
                className={cn(
                  'h-2 w-2 shrink-0 rounded-full',
                  connected ? 'bg-mint-500' : 'bg-coral-400',
                )}
              />
              <span className="truncate text-[11px] text-ink-400">
                {connected ? t('nav.connected') : t('nav.disconnected')}
              </span>
            </div>
            <span className="text-[11px] tabular text-ink-600">
              {version !== null ? `v${version}` : ''}
            </span>
          </div>
          {user !== null ? (
            <div className="mt-3 flex items-center gap-2.5 px-1">
              <span className="grid h-8 w-8 place-items-center rounded-full border border-mint-400/20 bg-mint-400/[0.08] text-[11px] font-semibold text-mint-300">
                {initials(user.display_name)}
              </span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[12px] font-medium text-ink-200">{user.display_name}</p>
                <p className="truncate text-[10px] uppercase tracking-wider text-ink-500">{user.role}</p>
              </div>
              <button
                type="button"
                onClick={() => {
                  void logout();
                }}
                className="rounded-lg p-2 text-ink-500 transition hover:bg-white/[0.06] hover:text-coral-300"
                aria-label={t('common.signOut')}
                title={t('common.signOut')}
              >
                <LogOut size={15} />
              </button>
            </div>
          ) : null}
        </div>
      </aside>
    </>
  );
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).slice(0, 2);
  return parts.map((part) => part.charAt(0).toUpperCase()).join('') || 'HS';
}
