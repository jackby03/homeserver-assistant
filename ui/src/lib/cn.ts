/**
 * Tiny class-name joiner. Falsy values are skipped so conditional classes read
 * naturally in JSX.
 */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ');
}
