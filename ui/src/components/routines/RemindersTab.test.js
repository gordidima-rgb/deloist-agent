import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RemindersTab from './RemindersTab.svelte';
import { planReminder } from '../../lib/reminders.js';

const NOW = new Date(2026, 10, 13, 12, 0); // 13 ноября 2026, 12:00

describe('planReminder', () => {
  it('makes a one-time reminder from date and time', () => {
    const plan = planReminder({ title: 'Позвонить маме', due_date: '2026-11-14', due_time: '18:00', recurrence: null }, NOW);
    expect(plan.fields).toEqual({ prompt: 'Позвонить маме', onceAt: new Date(2026, 10, 14, 18, 0).getTime() });
  });

  it('takes tomorrow when only a past time of today is given', () => {
    const plan = planReminder({ title: 'Таблетки', due_date: null, due_time: '09:00', recurrence: null }, NOW);
    expect(plan.fields.onceAt).toBe(new Date(2026, 10, 14, 9, 0).getTime());
  });

  it('turns a weekly rule into at + days', () => {
    const rule = { freq: 'weekly', interval: 1, weekdays: [3], monthday: null };
    const plan = planReminder({ title: 'Вынести мусор', due_time: '20:00', recurrence: rule }, NOW);
    expect(plan.fields).toEqual({ prompt: 'Вынести мусор', at: '20:00', days: [3], onceAt: null });
  });

  it('refuses what the timer cannot do', () => {
    expect(planReminder({ title: 'x', recurrence: { freq: 'monthly', interval: 1, weekdays: [], monthday: 5 }, due_time: '10:00' }, NOW).error).toBe('repeat');
    expect(planReminder({ title: 'x', due_date: '2026-11-14', recurrence: null }, NOW).error).toBe('time');
    expect(planReminder({ title: 'x', recurrence: null }, NOW).error).toBe('when');
    expect(planReminder({ title: 'x', due_date: '2026-11-12', due_time: '10:00', recurrence: null }, NOW).error).toBe('past');
  });
});

let host;
let instance;

async function drain() {
  for (let i = 0; i < 12; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const reminder = {
  id: 'rt_1',
  kind: 'reminder',
  prompt: 'Позвонить маме',
  enabled: true,
  onceAt: Date.now() + 3600_000,
  nextRun: Date.now() + 3600_000,
  leadMinutes: 15,
  channels: ['windows'],
  days: [],
  at: '',
  lastStatus: 'missed',
  lastRun: Date.now() - 1000,
};

describe('RemindersTab', () => {
  it('lists reminders with the missed status and tests one', async () => {
    const api = vi.fn((path) => {
      if (path === '/api/telegram') return Promise.resolve({ chatPaired: false });
      return Promise.resolve({ ok: true, deliveries: [{ channel: 'windows', ok: true, error: null }] });
    });
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(RemindersTab, { target: host, props: { api, routines: [reminder, { id: 'x', prompt: 'обычная', bot_id: 'b' }] } });
    await drain();
    expect(host.querySelectorAll('.rm-item')).toHaveLength(1);
    expect(host.textContent).toContain('пропущено: компьютер был выключен');
    // Telegram не привязан: фишка выключена, рядом ссылка на настройки.
    const telegramChip = [...host.querySelectorAll('.k-chip')].find((c) => c.textContent.trim() === 'Telegram');
    expect(telegramChip.disabled).toBe(true);
    [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Проверить').click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/reminders/rt_1/test', { method: 'POST' });
    expect(host.querySelector('.rm-test').textContent).toContain('отправлено');
  });
});
