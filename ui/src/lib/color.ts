/** Color helpers for the avatar renderer and charts. */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

const HEX_PATTERN = /^#?([0-9a-f]{6})$/i;
const SHORT_HEX = /^#?([0-9a-f]{3})$/i;

/** Parse `#RRGGBB` (or `RRGGBB`) into RGB; returns null when invalid. */
export function parseHexColor(value: string): Rgb | null {
  const trimmed = value.trim();
  const long = HEX_PATTERN.exec(trimmed);
  if (long !== null) {
    const hex = long[1];
    return {
      r: Number.parseInt(hex.slice(0, 2), 16),
      g: Number.parseInt(hex.slice(2, 4), 16),
      b: Number.parseInt(hex.slice(4, 6), 16),
    };
  }
  const short = SHORT_HEX.exec(trimmed);
  if (short !== null) {
    const [r, g, b] = short[1].split('');
    return {
      r: Number.parseInt(`${r}${r}`, 16),
      g: Number.parseInt(`${g}${g}`, 16),
      b: Number.parseInt(`${b}${b}`, 16),
    };
  }
  return null;
}

export function isValidHexColor(value: string): boolean {
  return parseHexColor(value) !== null;
}

function toHex(component: number): string {
  return Math.round(Math.min(255, Math.max(0, component)))
    .toString(16)
    .padStart(2, '0');
}

export function rgbToHex({ r, g, b }: Rgb): string {
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

/** Mix a color towards black (`amount` 0..1) or towards white (`amount` < 0). */
export function shade(value: string, amount: number): string {
  const rgb = parseHexColor(value);
  if (rgb === null) return value;
  if (amount >= 0) {
    return rgbToHex({
      r: rgb.r * (1 - amount),
      g: rgb.g * (1 - amount),
      b: rgb.b * (1 - amount),
    });
  }
  const lift = -amount;
  return rgbToHex({
    r: rgb.r + (255 - rgb.r) * lift,
    g: rgb.g + (255 - rgb.g) * lift,
    b: rgb.b + (255 - rgb.b) * lift,
  });
}

/** RGBA string with alpha, used for glows and gradients. */
export function withAlpha(value: string, alpha: number): string {
  const rgb = parseHexColor(value);
  if (rgb === null) return value;
  return `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${Math.min(1, Math.max(0, alpha))})`;
}

/** Choose a readable foreground color for a colored background. */
export function readableOn(value: string): string {
  const rgb = parseHexColor(value);
  if (rgb === null) return '#f2f7fb';
  const luminance = (0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b) / 255;
  return luminance > 0.6 ? '#0b1220' : '#f2f7fb';
}
