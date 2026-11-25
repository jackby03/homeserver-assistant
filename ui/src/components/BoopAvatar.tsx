import { createElement, useEffect, useRef } from 'react';
import type { Avatar, AvatarState, TrackingPoint } from '@/api/types';

interface BoopFaceElement extends HTMLElement {
  play?: (action: string) => unknown;
  reset?: () => unknown;
  startWaiting?: (options?: { variant?: string }) => Promise<unknown>;
  stopWaiting?: () => unknown;
  startListening?: () => Promise<unknown>;
  startSpeaking?: () => Promise<unknown>;
  setVoiceLevel?: (level: number | readonly [number, number]) => unknown;
  setPointerFollow?: (enabled?: boolean) => unknown;
}

interface BoopAvatarProps {
  avatar: Avatar;
  state: AvatarState;
  tracking?: TrackingPoint | null;
  size?: number;
  className?: string;
  /** Small decorative previews in lists/headers: no pointer interaction. */
  decorative?: boolean;
}

function actionForState(state: AvatarState): string {
  switch (state) {
    case 'profile_switch':
      return 'wink';
    case 'speaking':
      return 'success';
    case 'thinking':
      return 'inspect';
    case 'listening':
      return 'wary';
    default:
      return 'idle';
  }
}

/**
 * BoopFace 0.9 expressive avatar (MIT, vendored lite build).
 *
 * BoopFace renders the face as SVG and exposes an imperative API. The kiosk
 * maps its own avatar states to BoopFace actions so listening, thinking and
 * speaking are visibly different; the voice kit methods use the real lifecycle.
 *
 * `size` is passed only as the component attribute (not as a CSS width/height
 * pair) to avoid a ResizeObserver feedback loop that made the face flicker.
 */
export function BoopAvatar({
  avatar,
  state,
  size = 260,
  className,
  decorative = false,
}: BoopAvatarProps): React.JSX.Element {
  const ref = useRef<BoopFaceElement | null>(null);
  const ready = useRef(false);

  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const apply = (): void => {
      const face = ref.current;
      if (cancelled || face === null) return;
      try {
        face.stopWaiting?.();
      } catch {
        /* BoopFace API is best effort */
      }
      try {
        if (state === 'listening' && face.startListening !== undefined) {
          void face.startListening();
          return;
        }
        if (state === 'speaking' && face.startSpeaking !== undefined) {
          void face.startSpeaking();
          return;
        }
        if (state === 'thinking' && face.startWaiting !== undefined) {
          void face.startWaiting({ variant: 'bloom' });
          return;
        }
        if (state === 'idle') {
          face.reset?.();
          return;
        }
        void face.play?.(actionForState(state));
      } catch {
        /* never break the kiosk if the avatar API changes */
      }
    };

    timer = window.setTimeout(() => {
      if (ready.current) {
        apply();
        return;
      }
      void import('@/vendor/boopface/boopface-lite.js')
        .then(() => {
          ready.current = true;
          if (!cancelled) apply();
        })
        .catch(() => undefined);
    }, 40);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [state]);

  useEffect(() => {
    if (ref.current === null) return;
    ref.current.setPointerFollow?.(!decorative && state === 'tracking');
  }, [decorative, state]);

  return createElement('boop-face', {
    ref,
    size: Math.min(Math.max(32, Math.round(size)), 380),
    color: avatar.color,
    'eye-color': avatar.eye_color,
    'auto-sleep': '0',
    'pointer-follow': decorative ? 'false' : 'true',
    motion: decorative ? 'reduce' : 'auto',
    class: className,
    ...(decorative ? { 'aria-hidden': 'true' } : { 'aria-label': 'Bober' }),
  });
}
