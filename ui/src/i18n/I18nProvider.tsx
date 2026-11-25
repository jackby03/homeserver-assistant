import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { dictionaries, interpolate } from './dictionary';
import type { DictKey, Dictionary, UiLanguage } from './dictionary';
import { readStored, writeStored } from '@/lib/preferences';

export type { DictKey, UiLanguage } from './dictionary';

const LANGUAGE_KEY = 'hs.language';

interface I18nContextValue {
  language: UiLanguage;
  setLanguage: (language: UiLanguage) => void;
  t: (key: DictKey, vars?: Record<string, string | number>) => string;
}

const I18nContext = createContext<I18nContextValue | null>(null);

function isLanguage(value: string | null): value is UiLanguage {
  return value === 'es' || value === 'en';
}

function initialLanguage(): UiLanguage {
  const stored = readStored(LANGUAGE_KEY);
  if (isLanguage(stored)) return stored;
  return 'es';
}

export function I18nProvider({ children }: { children: ReactNode }): React.JSX.Element {
  const [language, setLanguageState] = useState<UiLanguage>(initialLanguage);

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  const setLanguage = useCallback((next: UiLanguage) => {
    setLanguageState(next);
    writeStored(LANGUAGE_KEY, next);
  }, []);

  const t = useCallback(
    (key: DictKey, vars?: Record<string, string | number>): string => {
      const dictionary: Dictionary = dictionaries[language];
      return interpolate(dictionary[key], vars);
    },
    [language],
  );

  const value = useMemo<I18nContextValue>(
    () => ({ language, setLanguage, t }),
    [language, setLanguage, t],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (context === null) throw new Error('useI18n must be used inside <I18nProvider>');
  return context;
}

/**
 * Map an unknown thrown value to a dictionary key for a localized message.
 * Known API error codes have dedicated copy; everything else falls back.
 */
export function errorMessageKey(error: unknown): DictKey {
  if (typeof error === 'object' && error !== null && 'code' in error) {
    const code = (error as { code: unknown }).code;
    if (typeof code === 'string') {
      switch (code) {
        case 'network_error':
          return 'errors.code.network_error';
        case 'unauthorized':
          return 'errors.code.unauthorized';
        case 'forbidden':
          return 'errors.code.forbidden';
        case 'not_found':
          return 'errors.code.not_found';
        case 'validation_error':
        case 'bad_request':
          return 'errors.code.validation_error';
        case 'conflict':
          return 'errors.code.conflict';
        case 'internal_error':
          return 'errors.code.internal_error';
        case 'invalid_response':
          return 'errors.code.invalid_response';
        default:
          return 'errors.generic';
      }
    }
  }
  return 'errors.generic';
}
