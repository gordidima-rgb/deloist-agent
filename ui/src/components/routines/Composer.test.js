import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import Composer from './Composer.svelte';
import { splitByMarks, removeKind } from '../../lib/composer.js';

const TEXT = 'позвонить маме завтра в 18:00';
// «завтра» — символы 15..21, «в 18:00» — 22..29.
const PARSED = {
  title: 'Позвонить маме',
  due_date: '2026-11-14',
  due_time: '18:00',
  recurrence: null,
  label: null,
  spans: [
    { start: 15, end: 21, kind: 'date' },
    { start: 22, end: 29, kind: 'time' },
  ],
  complete: true,
};

let host;
let handle;

beforeEach(() => {
  vi.useFakeTimers();
  host = document.createElement('div');
  document.body.append(host);
});

afterEach(() => {
  handle?.destroy();
  handle = null;
  host?.remove();
  vi.useRealTimers();
});

async function drain() {
  for (let i = 0; i < 10; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

async function typeAndWait(ms = 160) {
  await vi.advanceTimersByTimeAsync(ms);
  await drain();
}

const marks = () => [...host.querySelectorAll('.cmp-mark')].map((m) => [m.dataset.kind, m.textContent]);
const chips = () => [...host.querySelectorAll('.cmp-chip')].map((c) => [c.dataset.kind, c.querySelector('.cmp-chip-text').textContent]);

describe('Composer', () => {
  it('highlights the date and time parts from the spans', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(Composer, host, { value: TEXT, api });
    await typeAndWait();
    expect(api).toHaveBeenCalledTimes(1);
    expect(decodeURIComponent(api.mock.calls[0][0])).toContain(TEXT);
    expect(marks()).toEqual([
      ['date', 'завтра'],
      ['time', 'в 18:00'],
    ]);
    expect(chips().map((c) => c[0])).toEqual(['date', 'time']);
    expect(chips()[1][1]).toBe('18:00');
  });

  it('sends nothing while idle or empty', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(Composer, host, { value: '', api });
    await typeAndWait(2000);
    expect(api).not.toHaveBeenCalled();
    handle.update({ value: '   ' });
    await typeAndWait(2000);
    expect(api).not.toHaveBeenCalled();
    handle.update({ value: TEXT });
    await typeAndWait();
    expect(api).toHaveBeenCalledTimes(1);
    await typeAndWait(5000);
    expect(api).toHaveBeenCalledTimes(1);
  });

  it('waits for the pause in typing before asking', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(Composer, host, { value: 'п', api });
    await vi.advanceTimersByTimeAsync(100);
    handle.update({ value: 'по' });
    await vi.advanceTimersByTimeAsync(100);
    handle.update({ value: 'поз' });
    await typeAndWait(160);
    expect(api).toHaveBeenCalledTimes(1);
    expect(decodeURIComponent(api.mock.calls[0][0])).toContain('=поз');
  });

  it('ignores a stale response that arrives after a newer one', async () => {
    const waiting = [];
    const api = vi.fn(() => new Promise((resolve) => waiting.push(resolve)));
    handle = mountComponent(Composer, host, { value: 'ты', api });
    await typeAndWait();
    handle.update({ value: 'ты завтра' });
    await typeAndWait();
    expect(waiting).toHaveLength(2);
    // Новый ответ: без кусков. Старый приходит позже и «нашёл» дату там, где её уже нет.
    waiting[1]({ ...PARSED, spans: [], due_date: null, due_time: null });
    await drain();
    waiting[0]({ ...PARSED, spans: [{ start: 0, end: 2, kind: 'date' }], due_time: null });
    await drain();
    expect(marks()).toEqual([]);
    expect(chips()).toEqual([]);
  });

  it('removes a chip together with its words', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    handle = mountComponent(Composer, host, { value: TEXT, api });
    await typeAndWait();
    host.querySelector('.cmp-chip[data-kind="date"] .cmp-chip-x').click();
    await drain();
    expect(host.querySelector('textarea').value).toBe('позвонить маме в 18:00');
    // Текст изменился, значит снова идёт разбор; чип даты в ответе уже не придёт.
    api.mockResolvedValue({ ...PARSED, title: 'Позвонить маме', due_date: null, spans: [{ start: 15, end: 22, kind: 'time' }] });
    await typeAndWait();
    expect(chips().map((c) => c[0])).toEqual(['time']);
  });

  it('submits the parsed phrase on Enter', async () => {
    const api = vi.fn().mockResolvedValue(PARSED);
    const onsubmit = vi.fn();
    handle = mountComponent(Composer, host, { value: TEXT, api, onsubmit });
    await typeAndWait();
    host.querySelector('textarea').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await drain();
    expect(onsubmit).toHaveBeenCalledTimes(1);
    expect(onsubmit.mock.calls[0][0]).toMatchObject({ text: TEXT, title: 'Позвонить маме', due_date: '2026-11-14', due_time: '18:00' });
  });

  it('still submits when the parser is unreachable', async () => {
    const api = vi.fn().mockRejectedValue(new Error('offline'));
    const onsubmit = vi.fn();
    handle = mountComponent(Composer, host, { value: 'купить хлеб', api, onsubmit });
    await typeAndWait();
    host.querySelector('textarea').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await drain();
    expect(onsubmit.mock.calls[0][0]).toMatchObject({ text: 'купить хлеб', title: 'купить хлеб', due_date: null });
  });
});

describe('composer helpers', () => {
  it('splits by character numbers, not UTF-16 units', () => {
    const parts = splitByMarks('😀 завтра', [{ start: 2, end: 8, kind: 'date' }]);
    expect(parts).toEqual([
      { text: '😀 ', kind: null },
      { text: 'завтра', kind: 'date' },
    ]);
  });

  it('removes one kind and closes the gap', () => {
    expect(removeKind(TEXT, PARSED.spans, 'time')).toBe('позвонить маме завтра');
    expect(removeKind(TEXT, PARSED.spans, 'date')).toBe('позвонить маме в 18:00');
  });
});
