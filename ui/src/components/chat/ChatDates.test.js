import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import ChatDates from './ChatDates.svelte';

const TEXT = 'завтра в 10 +Бета\nсделать отчёт';
// «завтра» — символы 0..6, «в 10» — 7..11.
const PARSED = {
  title: 'сделать отчёт',
  due_date: '2026-11-17',
  due_time: '10:00',
  recurrence: null,
  label: null,
  spans: [
    { start: 0, end: 6, kind: 'date' },
    { start: 7, end: 11, kind: 'time' },
  ],
  complete: true,
};

let field;
let input;
let chipsHost;
let handle;

beforeEach(() => {
  vi.useFakeTimers();
  field = document.createElement('div');
  input = document.createElement('textarea');
  chipsHost = document.createElement('div');
  field.append(input);
  document.body.append(field, chipsHost);
});

afterEach(() => {
  handle?.destroy();
  handle = null;
  field.remove();
  chipsHost.remove();
  vi.useRealTimers();
});

async function drain() {
  for (let i = 0; i < 10; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

async function type(text) {
  input.value = text;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await vi.advanceTimersByTimeAsync(260);
  await drain();
}

const marks = () => [...field.querySelectorAll('.cd-mark')].map((m) => [m.dataset.kind, m.textContent]);
const chips = () => [...chipsHost.querySelectorAll('.cd-chip')].map((c) => [c.dataset.kind, c.querySelector('.cd-chip-text').textContent]);

describe('ChatDates', () => {
  it('highlights dates behind the chat field and shows chips under it', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(ChatDates, field, { input, chipsHost, api });
    await type(TEXT);
    expect(api).toHaveBeenCalledTimes(1);
    expect(decodeURIComponent(api.mock.calls[0][0])).toContain(TEXT);
    expect(marks()).toEqual([
      ['date', 'завтра'],
      ['time', 'в 10'],
    ]);
    expect(chips().map((c) => c[0])).toEqual(['date', 'time']);
    expect(input.value).toBe(TEXT); // текст сообщения не меняется
  });

  it('does not ask while the field is empty', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(ChatDates, field, { input, chipsHost, api });
    await type('   ');
    expect(api).not.toHaveBeenCalled();
  });

  it('follows the field when code clears it (send)', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(ChatDates, field, { input, chipsHost, api });
    await type(TEXT);
    input.value = '';
    input.dispatchEvent(new Event('cd-sync'));
    await vi.advanceTimersByTimeAsync(260);
    await drain();
    expect(marks()).toEqual([]);
    expect(chips()).toEqual([]);
  });

  it('a chip removes its words and keeps the line break', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(ChatDates, field, { input, chipsHost, api });
    await type(TEXT);
    chipsHost.querySelector('.cd-chip[data-kind="date"] .cd-chip-x').click();
    await drain();
    expect(input.value).toBe('в 10 +Бета\nсделать отчёт');
  });
});
