import { lazy, Suspense } from 'react';
import { BrowserRouter, Route, Routes } from 'react-router-dom';
import { I18nProvider } from '@/i18n/I18nProvider';
import { ToastProvider } from '@/state/ToastProvider';
import { AuthProvider } from '@/state/AuthProvider';
import { ToastViewport } from '@/components/Toast';
import { Spinner } from '@/components/Spinner';
import { AdminLayout } from '@/layout/AdminLayout';
import { RootRedirect, RequireAuth, SetupRoute } from '@/app/guards';
import { LoginPage } from '@/features/admin/LoginPage';
import { NotFoundPage } from '@/pages/NotFoundPage';

/* Route-level chunks keep the kiosk boot fast on the wall tablet. */
const DashboardPage = lazy(() =>
  import('@/features/admin/DashboardPage').then((module) => ({ default: module.DashboardPage })),
);
const InstancesPage = lazy(() =>
  import('@/features/admin/InstancesPage').then((module) => ({ default: module.InstancesPage })),
);
const AppsPage = lazy(() =>
  import('@/features/admin/AppsPage').then((module) => ({ default: module.AppsPage })),
);
const HomeAssistantPage = lazy(() =>
  import('@/features/admin/HomeAssistantPage').then((module) => ({
    default: module.HomeAssistantPage,
  })),
);
const NetworkPage = lazy(() =>
  import('@/features/admin/NetworkPage').then((module) => ({ default: module.NetworkPage })),
);
const AgentPage = lazy(() =>
  import('@/features/admin/AgentPage').then((module) => ({ default: module.AgentPage })),
);
const ProfilesPage = lazy(() =>
  import('@/features/admin/ProfilesPage').then((module) => ({ default: module.ProfilesPage })),
);
const AuditPage = lazy(() =>
  import('@/features/admin/AuditPage').then((module) => ({ default: module.AuditPage })),
);
const KioskPage = lazy(() =>
  import('@/features/kiosk/KioskPage').then((module) => ({ default: module.KioskPage })),
);

function RouteFallback(): React.JSX.Element {
  return (
    <div className="grid min-h-[60vh] place-items-center">
      <div className="flex items-center gap-3 text-sm text-ink-400">
        <Spinner />
        <span>…</span>
      </div>
    </div>
  );
}

/**
 * Single SPA with three experiences:
 * - `/setup` first-boot wizard (public only before initialization)
 * - `/login` + `/admin/*` management console
 * - `/kiosk` full-screen wall experience
 */
export function App(): React.JSX.Element {
  return (
    <I18nProvider>
      <ToastProvider>
        <AuthProvider>
          <BrowserRouter>
            <Suspense fallback={<RouteFallback />}>
              <Routes>
                <Route path="/" element={<RootRedirect />} />
                <Route path="/login" element={<LoginPage />} />
                <Route path="/setup" element={<SetupRoute />} />
                <Route path="/kiosk" element={<KioskPage />} />
                <Route
                  element={
                    <RequireAuth>
                      <AdminLayout />
                    </RequireAuth>
                  }
                >
                  <Route path="/admin" element={<RootRedirect />} />
                  <Route path="/admin/dashboard" element={<DashboardPage />} />
                  <Route path="/admin/instances" element={<InstancesPage />} />
                  <Route path="/admin/apps" element={<AppsPage />} />
                  <Route path="/admin/homeassistant" element={<HomeAssistantPage />} />
                  <Route path="/admin/network" element={<NetworkPage />} />
                  <Route path="/admin/agent" element={<AgentPage />} />
                  <Route path="/admin/profiles" element={<ProfilesPage />} />
                  <Route path="/admin/audit" element={<AuditPage />} />
                </Route>
                <Route path="*" element={<NotFoundPage />} />
              </Routes>
            </Suspense>
            <ToastViewport />
          </BrowserRouter>
        </AuthProvider>
      </ToastProvider>
    </I18nProvider>
  );
}
