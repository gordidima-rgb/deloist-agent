import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerProfile from './DrawerProfile.svelte';

let handle;
let host;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerProfile, host, { skills: [], levels: {}, ...props });
}

const waitFor = (check) =>
  vi.waitFor(() => {
    flushSync();
    check();
  });

const slider = () => host.querySelector('.dp-compact input[type="range"]');
const info = () => host.querySelector('#dp-compact-info').textContent.replace(/\s+/g, ' ');
const saveButton = () => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Сохранить');
const million = { known: true, window: 1000000, usable: 870000, min: 20, max: 95, step: 5, percent: 20, stored: false, currentTokens: 40000, mode: 'normal' };

function typeValue(input, value) {
  input.value = String(value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

afterEach(() => {
  handle?.destroy();
  host?.remove();
  handle = null;
  host = null;
});

describe('DrawerProfile compaction slider', () => {
  it('replaces the three-way select with a slider and shows tokens of the real window', async () => {
    mount({ bot: { id: 'b1', name: 'Кодер', compactAtPercent: 60 }, contextInfo: async () => ({ ...million, stored: true, percent: 60, currentTokens: 600000 }) });
    await waitFor(() => expect(info()).toContain('600'));
    expect(slider()).not.toBeNull();
    expect(host.querySelector('.dp-compact .k-label').textContent).toBe('Сжимать при 60% лимита');
    expect(info().replace(/\s/g, ' ')).toContain('≈ 600 000 из 1 000 000 токенов');
    expect(slider().getAttribute('aria-valuetext')).toBe('60% лимита');
    expect(host.textContent).not.toContain('Большая (для программирования)');
  });

  it('warns about cost above 200k tokens and not below', async () => {
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 60 }, contextInfo: async () => ({ ...million, stored: true, percent: 60, currentTokens: 600000 }) });
    await waitFor(() => expect(info()).toContain('600'));
    expect(host.querySelector('.dp-compact__warn')).not.toBeNull();
    typeValue(slider(), 20);
    expect(info().replace(/\s/g, ' ')).toContain('≈ 200 000 из 1 000 000');
    expect(host.querySelector('.dp-compact__warn')).toBeNull();
  });

  it('caps the shown tokens at the usable part of the window', async () => {
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 95 }, contextInfo: async () => ({ ...million, stored: true, percent: 95 }) });
    await waitFor(() => expect(info().replace(/\s/g, ' ')).toContain('Выше 870 000 токенов порог не поднимается'));
  });

  it('says so when the window is unknown and counts from the safe default', async () => {
    const unknown = { known: false, window: null, usable: 32000, min: 20, max: 95, step: 5, percent: 60, stored: true, currentTokens: 24000, mode: 'normal' };
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 60 }, contextInfo: async () => unknown });
    await waitFor(() => expect(info()).toContain('неизвестен'));
    expect(info().replace(/\s/g, ' ')).toContain('от 40 000 токенов: ≈ 24 000');
  });

  it('keeps the old default untouched until the slider moves and saves the percent only then', async () => {
    const onsave = vi.fn(async () => {});
    mount({ bot: { id: 'b1', name: 'К' }, contextInfo: async () => million, onsave });
    await waitFor(() => expect(info().replace(/\s/g, ' ')).toContain('Пока действует прежний режим: ≈ 40 000'));
    saveButton().click();
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
    expect(onsave.mock.calls[0][0]).not.toHaveProperty('compactAtPercent');
    expect(onsave.mock.calls[0][0]).not.toHaveProperty('contextMode');

    typeValue(slider(), 45);
    expect(host.querySelector('.dp-compact .k-label').textContent).toBe('Сжимать при 45% лимита');
    expect(info().replace(/\s/g, ' ')).toContain('≈ 450 000 из 1 000 000');
    await waitFor(() => expect(saveButton().disabled || saveButton().getAttribute('aria-busy') === 'true').toBe(false));
    saveButton().click();
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(2));
    expect(onsave.mock.calls[1][0].compactAtPercent).toBe(45);
  });

  it('starts the slider where the old mode keeps it', async () => {
    mount({ bot: { id: 'b1', name: 'К', contextMode: 'large' }, contextInfo: async () => ({ ...million, percent: 50, currentTokens: 500000, mode: 'large' }) });
    await waitFor(() => expect(slider().value).toBe('50'));
  });
});
