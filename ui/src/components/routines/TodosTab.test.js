import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import TodosTab from './TodosTab.svelte';
import TodoRecurrence from './TodoRecurrence.svelte';
import {
  groupBoard,
  filterCounts,
  carryOverCandidates,
  nextBucket,
  editorFromRecurrence,
  recurrenceFromEditor,
  weekdayOf,
  addDays,
} from '../../lib/todos.js';

const TODAY = '2026-11-13';

const todo = (over) => ({
  id: 't1',
  title: 'Позвонить маме',
  bucket: 'active',
  due_date: null,
  due_time: null,
  notes: '',
  recurrence: null,
  items: [],
  assignee: null,
  auto_close_after_min: null,
  done: false,
  done_at: null,
  archived: false,
  created_at: 1,
  ...over,
});

describe('board logic', () => {
  const list = [
    todo({ id: 'a', bucket: 'key', due_date: TODAY }),
    todo({ id: 'b', bucket: 'active', due_date: '2026-11-10' }),
    todo({ id: 'c', bucket: 'active' }),
    todo({ id: 'd', bucket: 'someday', done: true, due_date: TODAY }),
    todo({ id: 'e', bucket: 'active', due_date: '2026-11-01', recurrence: { freq: 'daily', interval: 1, weekdays: [], monthday: null } }),
  ];

  it('groups by section and splits done tasks off', () => {
    const board = groupBoard(list, { today: TODAY });
    expect(board.key.open.map((t) => t.id)).toEqual(['a']);
    expect(board.active.open.map((t) => t.id)).toEqual(['b', 'c', 'e']);
    expect(board.someday.open).toEqual([]);
    expect(board.someday.done.map((t) => t.id)).toEqual(['d']);
  });

  it('filters by due date and searches the title', () => {
    expect(groupBoard(list, { filter: 'today', today: TODAY }).key.open).toHaveLength(1);
    expect(groupBoard(list, { filter: 'overdue', today: TODAY }).active.open.map((t) => t.id)).toEqual(['b', 'e']);
    expect(groupBoard(list, { filter: 'nodue', today: TODAY }).active.open.map((t) => t.id)).toEqual(['c']);
    expect(groupBoard([todo({ id: 'x', title: 'Купить хлеб' }), todo({ id: 'y' })], { query: 'ХЛЕБ', today: TODAY }).active.open.map((t) => t.id)).toEqual(['x']);
  });

  it('counts open tasks per filter', () => {
    expect(filterCounts(list, TODAY)).toEqual({ all: 4, today: 1, overdue: 2, nodue: 1 });
  });

  it('carries over only open, overdue and non-repeating tasks', () => {
    expect(carryOverCandidates(list, TODAY).map((t) => t.id)).toEqual(['b']);
  });

  it('cycles the section of a new task', () => {
    expect(['key', 'active', 'someday', 'key'].map(nextBucket)).toEqual(['active', 'someday', 'key', 'active']);
  });

  it('does date arithmetic without the time zone', () => {
    expect(addDays('2026-11-30', 1)).toBe('2026-12-01');
    expect(weekdayOf('2026-11-13')).toBe(5); // пятница
  });
});

describe('recurrence editor output', () => {
  const none = { preset: 'none', weekdays: [], monthday: null };
  it('turns presets into server rules', () => {
    expect(recurrenceFromEditor(none)).toBe(null);
    expect(recurrenceFromEditor({ ...none, preset: 'daily' })).toEqual({ freq: 'daily', interval: 1, weekdays: [], monthday: null });
    expect(recurrenceFromEditor({ ...none, preset: 'weekdays' })).toEqual({ freq: 'weekly', interval: 1, weekdays: [1, 2, 3, 4, 5], monthday: null });
    expect(recurrenceFromEditor({ ...none, preset: 'biweekly', weekdays: [3, 1] })).toEqual({ freq: 'weekly', interval: 2, weekdays: [1, 3], monthday: null });
    expect(recurrenceFromEditor({ ...none, preset: 'weekly' }, '2026-11-13').weekdays).toEqual([5]);
    expect(recurrenceFromEditor({ ...none, preset: 'monthly', monthday: 15 })).toEqual({ freq: 'monthly', interval: 1, weekdays: [], monthday: 15 });
    expect(recurrenceFromEditor({ ...none, preset: 'yearly' }).freq).toBe('yearly');
  });

  it('recognises the preset of a stored rule', () => {
    expect(editorFromRecurrence(null).preset).toBe('none');
    expect(editorFromRecurrence({ freq: 'weekly', interval: 1, weekdays: [1, 2, 3, 4, 5], monthday: null }).preset).toBe('weekdays');
    expect(editorFromRecurrence({ freq: 'weekly', interval: 2, weekdays: [2], monthday: null }).preset).toBe('biweekly');
    expect(editorFromRecurrence({ freq: 'daily', interval: 3, weekdays: [], monthday: null }).preset).toBe('custom');
  });
});

let host;
let instance;

async function drain() {
  for (let i = 0; i < 14; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}

function render(Component, props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Component, { target: host, props });
  return drain();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('TodoRecurrence component', () => {
  it('emits a weekly rule with the picked days', async () => {
    const onchange = vi.fn();
    await render(TodoRecurrence, { rule: null, dueDate: '2026-11-13', onchange });
    const select = host.querySelector('select');
    select.value = 'weekly';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await drain();
    expect(onchange).toHaveBeenLastCalledWith({ freq: 'weekly', interval: 1, weekdays: [5], monthday: null });
    const days = [...host.querySelectorAll('.k-chip')];
    expect(days).toHaveLength(7);
    days[0].click(); // понедельник
    await drain();
    expect(onchange).toHaveBeenLastCalledWith({ freq: 'weekly', interval: 1, weekdays: [1, 5], monthday: null });
  });
});

describe('TodosTab', () => {
  // Вкладка берёт «сегодня» из часов: держим их на TODAY, иначе на следующий день счётчик «Просрочено» поедет.
  // Подменяем только Date — таймеры остаются настоящими.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(`${TODAY}T12:00:00`));
  });
  afterEach(() => vi.useRealTimers());

  const stored = [
    todo({ id: 'k1', title: 'Сдать отчёт', bucket: 'key', due_date: TODAY }),
    todo({ id: 'a1', title: 'Старое дело', bucket: 'active', due_date: '2026-01-01' }),
    todo({ id: 'a2', title: 'Купить хлеб', bucket: 'active' }),
  ];
  const apiFor = (extra = () => undefined) =>
    vi.fn((path, opts) => {
      const own = extra(path, opts);
      if (own !== undefined) return own;
      if (path.startsWith('/api/todos?')) return Promise.resolve(stored);
      if (path.startsWith('/api/dates/parse')) return Promise.resolve({ title: 'x', spans: [] });
      return Promise.resolve({});
    });
  const rowOf = (title) => [...host.querySelectorAll('.td-row')].find((r) => r.textContent.includes(title));

  it('shows the board, the overdue count and the carry-over strip', async () => {
    await render(TodosTab, { api: apiFor() });
    const sections = [...host.querySelectorAll('.td-section')];
    expect(sections).toHaveLength(3);
    expect(sections[0].textContent).toContain('Сдать отчёт');
    expect(sections[1].textContent).toContain('Купить хлеб');
    expect(host.querySelector('.td-filters').textContent).toContain('Просрочено (1)');
    expect(host.querySelector('.td-carry').textContent).toContain('С прошлых дней осталось 1');
  });

  it('checks a task at once and puts it back when the server refuses', async () => {
    let refuse;
    const api = apiFor((path, opts) => {
      if (path === '/api/todos/a2/done' && opts?.method === 'POST') return new Promise((_, reject) => (refuse = () => reject(new Error('boom'))));
    });
    await render(TodosTab, { api });
    const check = () => rowOf('Купить хлеб')?.querySelector('.td-check');
    check().click();
    await drain();
    // сразу ушла в «Выполнено»: в списке открытых её уже нет
    expect(rowOf('Купить хлеб')).toBeUndefined();
    expect(host.querySelector('.td-done-toggle').textContent).toContain('Выполнено (1)');
    refuse();
    await drain();
    expect(check().getAttribute('aria-checked')).toBe('false');
    expect(host.querySelector('.td-error').textContent).toContain('boom');
  });

  it('moves a task to another section from the menu', async () => {
    const api = apiFor();
    await render(TodosTab, { api });
    rowOf('Купить хлеб').querySelector('[data-menu-trigger]').click();
    await drain();
    const item = [...host.querySelectorAll('.k-menu__item')].find((b) => b.textContent.includes('Когда-нибудь'));
    item.click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/todos/a2', { method: 'PATCH', body: { bucket: 'someday' } });
    expect(host.querySelectorAll('.td-section')[2].textContent).toContain('Купить хлеб');
  });

  it('carries overdue tasks over to today', async () => {
    const api = apiFor();
    await render(TodosTab, { api });
    [...host.querySelectorAll('.td-carry button')][0].click();
    await drain();
    expect(api).toHaveBeenCalledWith('/api/todos/carry-over', { method: 'POST' });
  });
});
