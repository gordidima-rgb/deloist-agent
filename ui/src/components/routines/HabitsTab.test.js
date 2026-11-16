import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import HabitsTab from './HabitsTab.svelte';
import { daysFromMask, maskFromDays } from '../../lib/habits.js';

const habit = {
  id: 'h1',
  title: 'Выпить стакан воды',
  caption: '',
  icon: 'drop',
  days_mask: 127,
  today_done: false,
  today_scheduled: true,
  streak: 2,
  week: [],
  week_done: 3,
  week_scheduled: 7,
};

let host;
let instance;

async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

async function render(api) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(HabitsTab, { target: host, props: { api } });
  await drain();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

const check = () => host.querySelector('.hb-check');

describe('HabitsTab', () => {
  it('shows streak and week from the server', async () => {
    await render(vi.fn().mockResolvedValue([habit]));
    expect(host.querySelector('.hb-title').textContent).toBe('Выпить стакан воды');
    expect(host.querySelector('.hb-meta').textContent).toContain('Серия: 2 дн.');
    expect(host.querySelector('.hb-meta').textContent).toContain('3 из 7 за неделю');
    expect(check().getAttribute('aria-checked')).toBe('false');
  });

  it('checks the habit at once and keeps it when the server agrees', async () => {
    let answer;
    const api = vi.fn((path, opts) => {
      if (path === '/api/habits') return Promise.resolve([habit]);
      return new Promise((resolve) => {
        answer = () => resolve({ ...habit, today_done: true, streak: 3, week_done: 4 });
      });
    });
    await render(api);
    check().click();
    await drain();
    // Ответа ещё нет, а отметка уже стоит.
    expect(check().getAttribute('aria-checked')).toBe('true');
    expect(api).toHaveBeenCalledWith('/api/habits/h1/check', { method: 'POST' });
    answer();
    await drain();
    expect(check().getAttribute('aria-checked')).toBe('true');
    expect(host.querySelector('.hb-meta').textContent).toContain('Серия: 3 дн.');
  });

  it('undoes the check and shows the error when the server refuses', async () => {
    const api = vi.fn((path) => (path === '/api/habits' ? Promise.resolve([habit]) : Promise.reject(Object.assign(new Error('нет места'), { status: 500 }))));
    await render(api);
    check().click();
    await drain();
    expect(check().getAttribute('aria-checked')).toBe('false');
    expect(host.querySelector('.hb-meta').textContent).toContain('Серия: 2 дн.');
    expect(host.querySelector('[role="alert"]').textContent).toContain('нет места');
  });

  it('removes a check with DELETE', async () => {
    const done = { ...habit, today_done: true };
    const api = vi.fn((path, opts) => (path === '/api/habits' ? Promise.resolve([done]) : Promise.resolve({ ...habit })));
    await render(api);
    expect(check().getAttribute('aria-checked')).toBe('true');
    check().click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/habits/h1/check', { method: 'DELETE' });
    expect(check().getAttribute('aria-checked')).toBe('false');
  });

  it('reloads on habits_changed', async () => {
    const api = vi.fn().mockResolvedValue([habit]);
    await render(api);
    const before = api.mock.calls.length;
    window.dispatchEvent(new CustomEvent('deloist:event', { detail: { type: 'habits_changed' } }));
    await drain();
    expect(api.mock.calls.length).toBe(before + 1);
  });
});

describe('habit day mask', () => {
  it('round-trips weekdays through the mask', () => {
    expect(maskFromDays(daysFromMask(31))).toBe(31);
    expect(daysFromMask(42)).toEqual([false, true, false, true, false, true, false]);
    expect(maskFromDays(Array(7).fill(true))).toBe(127);
  });
});

describe('HabitsTab reminder time', () => {
  const remind = { ...habit, remind_at: '09:30', remind_channels: ['windows'] };
  const telegramOff = (path, body) => {
    if (path === '/api/habits') return Promise.resolve([remind]);
    if (path === '/api/telegram') return Promise.resolve({ chatPaired: false });
    return Promise.resolve({ ...remind, ...body });
  };

  const editButton = () => [...host.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Изменить');
  const saveButton = () => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Сохранить');
  const chip = (name) => [...host.querySelectorAll('.k-chip')].find((c) => c.textContent.trim() === name);
  function setTime(value) {
    const input = host.querySelector('input[type="time"]');
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it('shows the bell with the time and opens the form with the saved channels', async () => {
    await render(vi.fn(telegramOff));
    expect(host.querySelector('.hb-bell').textContent).toContain('09:30');
    editButton().click();
    await drain();
    expect(host.querySelector('input[type="time"]').value).toBe('09:30');
    expect(chip('Windows').getAttribute('aria-pressed')).toBe('true');
    // Telegram не привязан: фишка выключена.
    expect(chip('Telegram').disabled).toBe(true);
  });

  it('saves the time and the channels with the habit', async () => {
    const api = vi.fn(telegramOff);
    await render(api);
    editButton().click();
    await drain();
    setTime('08:15');
    await drain();
    host.querySelector('form.hb-form').dispatchEvent(new Event('submit', { cancelable: true }));
    await drain();
    expect(api).toHaveBeenCalledWith('/api/habits/h1', {
      method: 'PATCH',
      body: { title: 'Выпить стакан воды', icon: 'drop', days_mask: 127, remind_at: '08:15', remind_channels: ['windows'] },
    });
  });

  it('keeps save off while a reminder has no channel, and an empty time sends no channels', async () => {
    const api = vi.fn(telegramOff);
    await render(api);
    editButton().click();
    await drain();
    chip('Windows').click();
    await drain();
    expect(host.querySelector('.hb-hint').textContent).toContain('Выбери, куда прислать напоминание.');
    expect(saveButton().disabled).toBe(true);
    chip('Windows').click();
    await drain();
    // Время очищено: напоминание выключено, каналы не уходят.
    host.querySelector('.hb-link').click();
    await drain();
    expect(saveButton().disabled).toBe(false);
    host.querySelector('form.hb-form').dispatchEvent(new Event('submit', { cancelable: true }));
    await drain();
    expect(api).toHaveBeenCalledWith('/api/habits/h1', { method: 'PATCH', body: { title: 'Выпить стакан воды', icon: 'drop', days_mask: 127, remind_at: '' } });
  });
});
