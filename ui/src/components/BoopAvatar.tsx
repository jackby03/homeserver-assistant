import { createElement, useEffect, useRef } from 'react';
import type { Avatar, AvatarState, TrackingPoint } from '@/api/types';
import type { CSSProperties } from 'react';

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
 * speaking are visibly different; the voice kit methods are used when present.
 */
export function BoopAvatar({
  avatar,
  state,
  size = 260,
  className,
}: BoopAvatarProps): React.JSX.Element {
  const ref = useRef<BoopFaceElement | null>(null);
  const ready = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const apply = (): void => {
      if (cancelled || ref.current === null) return;
      const face = ref.current;
      const stop = (): void => {
        try {
          face.stopWaiting?.();
        } catch {
          /* not supported */
        }
      };
      stop();
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
        /* BoopFace API is best effort; never break the kiosk */
      }
    };

    if (ready.current) {
      apply();
      return () => {
        cancelled = true;
      };
    }
    void import('@/vendor/boopface/boopface-lite.js')
      .then(() => {
        ready.current = true;
        if (!cancelled) apply();
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [state]);

  useEffect(() => {
    if (ref.current === null) return;
    ref.current.setPointerFollow?.(state === 'tracking');
  }, [state]);

  const style: CSSProperties = { width: size, height: size };
  const faceClass = className === undefined ? undefined : className;
  return createElement(
    'boop-face',
    {
      ref,
      size,
      color: avatar.color,
      'eye-color': avatar.eye_color,
      'auto-sleep': '0',
      motion: 'auto',
      class: faceClass,
      style,
      'aria-label': 'Aurora',
    },
    null,
  );
}

