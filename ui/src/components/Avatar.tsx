import { useEffect, useId, useState } from 'react';
import type { CSSProperties } from 'react';
import type { Avatar, AvatarShape, AvatarState, TrackingPoint } from '@/api/types';
import type { DictKey } from '@/i18n/dictionary';
import { useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { isValidHexColor, shade, withAlpha } from '@/lib/color';
import { clamp } from '@/lib/format';
import { useReducedMotion } from '@/lib/preferences';

/**
 * Distinct silhouettes, all designed inside the 200x200 viewBox so the face
 * stays centered regardless of the shape.
 */
const SHAPE_PATHS: Record<AvatarShape, string> = {
  circle: 'M100 27 A78 78 0 1 0 100 183 A78 78 0 1 0 100 27 Z',
  squircle:
    'M100 27 C148 27 178 57 178 105 C178 153 148 183 100 183 C52 183 22 153 22 105 C22 57 52 27 100 27 Z',
  cloud:
    'M64 158 L136 158 C161 158 179 141 178 119 C177 101 164 88 147 87 C143 67 124 54 104 58 C92 42 66 46 57 67 C37 66 22 84 24 105 C13 116 16 135 29 145 C35 152 49 158 64 158 Z',
  cone: 'M100 28 C110 28 117 34 123 47 L172 151 C180 165 171 177 155 177 L45 177 C29 177 20 165 28 151 L77 47 C83 34 90 28 100 28 Z',
  droplet:
    'M100 24 C100 24 167 96 167 131 C167 165 137 185 100 185 C63 185 33 165 33 131 C33 96 100 24 100 24 Z',
};

const SHAPE_LABEL_KEYS: Record<AvatarShape, DictKey> = {
  circle: 'avatar.shape.circle',
  squircle: 'avatar.shape.squircle',
  cloud: 'avatar.shape.cloud',
  cone: 'avatar.shape.cone',
  droplet: 'avatar.shape.droplet',
};

const STATE_LABEL_KEYS: Record<AvatarState, DictKey> = {
  idle: 'avatar.state.idle',
  tracking: 'avatar.state.tracking',
  listening: 'avatar.state.listening',
  thinking: 'avatar.state.thinking',
  speaking: 'avatar.state.speaking',
  profile_switch: 'avatar.state.profile_switch',
};

/** CSS modifier class for an avatar state (unit tested). */
export function avatarStateClass(state: AvatarState): string {
  return `is-${state}`;
}

interface AvatarFigureProps {
  avatar: Avatar;
  state: AvatarState;
  tracking?: TrackingPoint | null;
  size?: number;
  className?: string;
  /** Hide from assistive tech when decorative (e.g. behind a name). */
  decorative?: boolean;
}

/**
 * Premium SVG avatar engine.
 *
 * States are driven by CSS classes on the root element; gaze and eye tilt are
 * inline SVG transforms so tracking stays smooth without re-rendering parents.
 */
export function AvatarFigure({
  avatar,
  state,
  tracking = null,
  size = 240,
  className,
  decorative = false,
}: AvatarFigureProps): React.JSX.Element {
  const { t } = useI18n();
  const reducedMotion = useReducedMotion();
  const rawId = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const [blinking, setBlinking] = useState(false);

  const baseColor = isValidHexColor(avatar.color) ? avatar.color : '#6ee7ff';
  const eyeColor = isValidHexColor(avatar.eye_color) ? avatar.eye_color : '#0b1220';
  const tilt = clamp(Number.isFinite(avatar.eye_tilt_deg) ? avatar.eye_tilt_deg : 0, -45, 45);

  useEffect(() => {
    if (reducedMotion) {
      setBlinking(false);
      return undefined;
    }
    let closeTimer = 0;
    let scheduleTimer = 0;
    const schedule = (): void => {
      const delay = 2_600 + Math.random() * 4_200;
      scheduleTimer = window.setTimeout(() => {
        setBlinking(true);
        closeTimer = window.setTimeout(() => {
          setBlinking(false);
          schedule();
        }, 160);
      }, delay);
    };
    schedule();
    return () => {
      window.clearTimeout(scheduleTimer);
      window.clearTimeout(closeTimer);
    };
  }, [reducedMotion]);

  const hasGaze = tracking !== null && Number.isFinite(tracking.x) && Number.isFinite(tracking.y);
  const confidence = hasGaze ? clamp(tracking.confidence, 0, 1) : 0;
  const gazeScale = state === 'tracking' ? 1 : 0.55;
  const gazeX = hasGaze ? clamp((tracking.x - 0.5) * 9, -5, 5) * gazeScale * (0.4 + confidence * 0.6) : 0;
  const gazeY = hasGaze ? clamp((tracking.y - 0.5) * 6, -3.5, 3.5) * gazeScale * (0.4 + confidence * 0.6) : 0;

  const style: CSSProperties = { width: size, height: size };
  const ariaLabel = decorative
    ? undefined
    : `${t(SHAPE_LABEL_KEYS[avatar.shape])}, ${t(STATE_LABEL_KEYS[state])}`;

  return (
    <svg
      viewBox="0 0 200 200"
      className={cn('avatar-root', avatarStateClass(state), className)}
      style={style}
      role={decorative ? 'presentation' : 'img'}
      aria-label={ariaLabel}
      aria-hidden={decorative ? true : undefined}
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <radialGradient id={`${rawId}-body`} cx="34%" cy="24%" r="84%">
          <stop offset="0%" stopColor={shade(baseColor, -0.32)} />
          <stop offset="52%" stopColor={baseColor} />
          <stop offset="100%" stopColor={shade(baseColor, 0.58)} />
        </radialGradient>
        <radialGradient id={`${rawId}-glow`}>
          <stop offset="0%" stopColor={baseColor} stopOpacity="0.35" />
          <stop offset="100%" stopColor={baseColor} stopOpacity="0" />
        </radialGradient>
        <linearGradient id={`${rawId}-sheen`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#ffffff" stopOpacity="0.32" />
          <stop offset="100%" stopColor="#ffffff" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* Ambient floor glow. */}
      <ellipse cx="100" cy="160" rx="92" ry="30" fill={`url(#${rawId}-glow)`} opacity="0.85" />

      {/* Listening rings. */}
      <circle
        className="avatar-ring avatar-ring--a"
        cx="100"
        cy="108"
        r="80"
        fill="none"
        stroke={withAlpha(baseColor, 0.55)}
        strokeWidth="2"
      />
      <circle
        className="avatar-ring avatar-ring--b"
        cx="100"
        cy="108"
        r="80"
        fill="none"
        stroke={withAlpha(baseColor, 0.35)}
        strokeWidth="1.5"
      />

      {/* Speaking ripples. */}
      <circle
        className="avatar-ripple avatar-ripple--a"
        cx="100"
        cy="110"
        r="74"
        fill="none"
        stroke={withAlpha(baseColor, 0.35)}
        strokeWidth="2"
      />
      <circle
        className="avatar-ripple avatar-ripple--b"
        cx="100"
        cy="110"
        r="74"
        fill="none"
        stroke={withAlpha(baseColor, 0.22)}
        strokeWidth="1.5"
      />

      <g className="avatar-body-group">
        <path
          d={SHAPE_PATHS[avatar.shape]}
          fill={`url(#${rawId}-body)`}
          stroke={withAlpha(baseColor, 0.5)}
          strokeWidth="1.2"
          strokeLinejoin="round"
        />
        <ellipse
          cx="72"
          cy="62"
          rx="34"
          ry="21"
          fill={`url(#${rawId}-sheen)`}
          transform="rotate(-26 72 62)"
        />
        <path
          d="M46 156 C60 172 140 172 154 156"
          fill="none"
          stroke={withAlpha(shade(baseColor, 0.35), 0.55)}
          strokeWidth="1"
          strokeLinecap="round"
        />

        <g transform={`rotate(${tilt} 100 108)`}>
          <g
            className="avatar-eyes"
            style={{ transform: `translate(${gazeX.toFixed(2)}px, ${gazeY.toFixed(2)}px)` }}
          >
            <g className={cn('avatar-blink-layer', blinking && 'avatar-eye-blink')}>
              <rect x="57" y="88" width="22" height="36" rx="11" fill={eyeColor} />
              <rect x="121" y="88" width="22" height="36" rx="11" fill={eyeColor} />
              <circle cx="64.5" cy="97" r="3.1" fill="#ffffff" opacity="0.5" />
              <circle cx="128.5" cy="97" r="3.1" fill="#ffffff" opacity="0.5" />
            </g>
          </g>
          <g className="avatar-brows" aria-hidden="true">
            <path
              className="avatar-brow avatar-brow--left"
              d="M55 77 Q67 71 80 77"
              fill="none"
              stroke={eyeColor}
              strokeWidth="4"
              strokeLinecap="round"
              opacity="0.62"
            />
            <path
              className="avatar-brow avatar-brow--right"
              d="M120 77 Q133 71 145 77"
              fill="none"
              stroke={eyeColor}
              strokeWidth="4"
              strokeLinecap="round"
              opacity="0.62"
            />
          </g>
          <circle
            className="avatar-cheek avatar-cheek--left"
            cx="51"
            cy="124"
            r="8"
            fill={withAlpha(baseColor, 0.65)}
          />
          <circle
            className="avatar-cheek avatar-cheek--right"
            cx="149"
            cy="124"
            r="8"
            fill={withAlpha(baseColor, 0.65)}
          />
          <rect
            className="avatar-mouth"
            x="88"
            y="140"
            width="24"
            height="9"
            rx="4.5"
            fill={eyeColor}
            opacity="0.82"
          />
        </g>
      </g>

      {/* Thinking orbit: three dots circling the body. */}
      <g className="avatar-orbit" aria-hidden="true">
        <circle className="avatar-orbit-dot" cx="100" cy="42" r="2.8" fill={shade(baseColor, -0.2)} />
        <circle
          className="avatar-orbit-dot"
          cx="156"
          cy="74"
          r="2.4"
          fill={shade(baseColor, -0.35)}
          style={{ animationDelay: '0.25s' }}
        />
        <circle
          className="avatar-orbit-dot"
          cx="44"
          cy="74"
          r="2.4"
          fill={shade(baseColor, -0.1)}
          style={{ animationDelay: '0.5s' }}
        />
      </g>
    </svg>
  );
}
