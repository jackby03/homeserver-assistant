import { useEffect, useState } from 'react';
import { Outlet } from 'react-router-dom';
import { authApi } from '@/api/auth';
import { useResource } from '@/lib/use-resource';
import { Sidebar } from './Sidebar';
import { Topbar } from './Topbar';
import { CommandPalette } from './CommandPalette';

/** Authenticated admin shell: glass sidebar, sticky topbar, routed content. */
export function AdminLayout(): React.JSX.Element {
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const health = useResource(() => authApi.health(), [], { pollMs: 15_000 });

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLocaleLowerCase() === 'k') {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  return (
    <div className="flex min-h-screen">
      <Sidebar
        mobileOpen={mobileOpen}
        onCloseMobile={() => setMobileOpen(false)}
        connected={health.data !== null && health.error === null}
        version={health.data?.version ?? null}
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar
          onOpenMenu={() => setMobileOpen(true)}
          onOpenPalette={() => setPaletteOpen(true)}
        />
        <main className="mx-auto w-full max-w-[1440px] flex-1 px-4 pb-20 pt-6 sm:px-6 lg:px-8">
          <div className="animate-fade-up">
            <Outlet />
          </div>
        </main>
      </div>
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
    </div>
  );
}
