import { describe, it, expect } from 'vitest';
import { shouldShowSignIn } from './signIn.js';

const a = { id: 'a', createdAt: 1, skills: [] };
const b = { id: 'b', createdAt: 2, skills: [] };

describe('shouldShowSignIn', () => {
  it('shows for the first bot', () => {
    expect(shouldShowSignIn(a, [b, a])).toBe(true);
    expect(shouldShowSignIn(b, [b, a])).toBe(false);
  });
  it('shows for a bot with the free-models skill', () => {
    expect(shouldShowSignIn({ ...b, skills: ['skill.free-models'] }, [a, b])).toBe(true);
  });
  it('hides once done or postponed', () => {
    expect(shouldShowSignIn({ ...a, googleSignIn: 'done' }, [a])).toBe(false);
    expect(shouldShowSignIn({ ...a, googleSignIn: 'later' }, [a])).toBe(false);
  });
  it('hides when the bot works in the person own Chrome', () => {
    expect(shouldShowSignIn({ ...a, browserMode: 'mine' }, [a])).toBe(false);
  });
});
