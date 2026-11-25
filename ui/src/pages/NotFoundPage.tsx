import { Link } from 'react-router-dom';
import { Compass } from 'lucide-react';
import { BrandMark } from '@/components/BrandMark';
import { EmptyState } from '@/components/EmptyState';
import { useI18n } from '@/i18n/I18nProvider';
import { useAuth } from '@/state/AuthProvider';

/** Fallback route for unknown paths. */
export function NotFoundPage(): React.JSX.Element {
  const { t } = useI18n();
  const { status } = useAuth();
  const backTo = status === 'authenticated' ? '/admin/dashboard' : '/login';

  return (
    <div className="grid min-h-screen place-items-center px-6">
      <div className="w-full max-w-lg space-y-6">
        <div className="flex justify-center">
          <BrandMark size={40} withWordmark />
        </div>
        <EmptyState
          icon={<Compass size={20} />}
          title={t('errors.notFound')}
          body={t('app.tagline')}
          action={
            <Link to={backTo} className="btn btn-primary">
              {t('common.back')}
            </Link>
          }
        />
      </div>
    </div>
  );
}
