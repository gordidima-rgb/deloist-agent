import { describe, expect, it, vi } from 'vitest';
import { channelsOf, flagsOf, isTelegramPaired } from './channels.js';

describe('channelsOf', () => {
  it('keeps telegram only when the chat is paired', () => {
    expect(channelsOf({ windows: true, telegram: true, paired: false })).toEqual(['windows']);
    expect(channelsOf({ windows: true, telegram: true, paired: true })).toEqual(['windows', 'telegram']);
    expect(channelsOf({ windows: false, telegram: true, paired: false })).toEqual([]);
  });
});

describe('flagsOf', () => {
  it('reads the flags and treats an empty list as windows', () => {
    expect(flagsOf([])).toEqual({ windows: true, telegram: false });
    expect(flagsOf(undefined)).toEqual({ windows: true, telegram: false });
    expect(flagsOf(['telegram'])).toEqual({ windows: false, telegram: true });
  });
});

describe('isTelegramPaired', () => {
  it('asks the server and counts a failure as not paired', async () => {
    const paired = vi.fn().mockResolvedValue({ chatPaired: true });
    expect(await isTelegramPaired(paired)).toBe(true);
    expect(paired).toHaveBeenCalledWith('/api/telegram');
    expect(await isTelegramPaired(vi.fn().mockRejectedValue(new Error('нет сети')))).toBe(false);
  });
});
