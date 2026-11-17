import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, tick } from 'svelte';
import { toast } from '../../../lib/toast.js';

afterEach(() => {
  vi.useRealTimers();
});

describe('toast', () => {
  it('shows a message and hides it after its duration', async () => {
    vi.useFakeTimers();
    const id = toast('Профиль сохранён');
    flushSync();
    await tick();
    flushSync();

    const first = document.querySelector('.k-toast');
    expect(first).not.toBeNull();
    expect(first.textContent).toContain('Профиль сохранён');

    vi.advanceTimersByTime(5000);
    flushSync();
    await tick();
    flushSync();

    expect(document.querySelector('.k-toast')).toBeNull();
    expect(typeof id).toBe('number');
  });
});
