import { describe, expect, it } from 'vitest';
import { dictionaries, interpolate } from './dictionary';

describe('i18n interpolation', () => {
  it('replaces double-brace placeholders used by the dictionaries', () => {
    expect(interpolate('Buenas noches, {{name}}', { name: 'admin' })).toBe(
      'Buenas noches, admin',
    );
    expect(interpolate('{{count}} dispositivos', { count: 4 })).toBe('4 dispositivos');
    expect(interpolate('{{free}} libres de {{total}}', { free: '1 GB', total: '2 GB' })).toBe(
      '1 GB libres de 2 GB',
    );
  });

  it('keeps single-brace templates working', () => {
    expect(interpolate('Hola {name}', { name: 'Casa' })).toBe('Hola Casa');
  });

  it('leaves unknown placeholders visible', () => {
    expect(interpolate('Hola {{name}}', {})).toBe('Hola {{name}}');
  });

  it('returns the template untouched without variables', () => {
    expect(interpolate('Sin variables')).toBe('Sin variables');
  });

  it('has the same keys in Spanish and English', () => {
    const spanish = Object.keys(dictionaries.es).sort();
    const english = Object.keys(dictionaries.en).sort();
    expect(spanish).toEqual(english);
  });
});
