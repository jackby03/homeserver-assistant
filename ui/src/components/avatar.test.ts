import { describe, expect, it } from 'vitest';
import type { AvatarState } from '@/api/types';
import { avatarStateClass } from '@/components/Avatar';

describe('avatarStateClass', () => {
  it('maps every avatar state to a stable CSS modifier class', () => {
    const cases: Array<[AvatarState, string]> = [
      ['idle', 'is-idle'],
      ['tracking', 'is-tracking'],
      ['listening', 'is-listening'],
      ['thinking', 'is-thinking'],
      ['speaking', 'is-speaking'],
      ['profile_switch', 'is-profile_switch'],
    ];
    for (const [state, expected] of cases) {
      expect(avatarStateClass(state)).toBe(expected);
    }
  });
});
