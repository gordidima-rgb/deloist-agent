import { describe, expect, it } from 'vitest';
import { editorFromRecurrence, recurrenceFromEditor, recurrenceSummary } from './todos.js';
import { onceMoment, scheduleFields, scheduleFromRoutine, scheduleProblem, timedScheduleText } from './routineSchedule.js';

const t = (text, vars) => (vars ? text.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : text);
const third = { freq: 'yearly', interval: 1, weekdays: [6], monthday: null, nth: 3, month: 9 };

describe('rule <-> editor', () => {
  it('builds the 3rd Saturday of September and reads it back', () => {
    const editor = { preset: 'yearly', weekdays: [], monthday: null, by: 'nth', nth: 3, nthDay: 6, month: 9 };
    expect(recurrenceFromEditor(editor)).toEqual(third);
    expect(editorFromRecurrence(third)).toMatchObject({ preset: 'yearly', by: 'nth', nth: 3, nthDay: 6, month: 9 });
  });

  it('builds the last weekday of every month and a fixed yearly date', () => {
    const monthly = { preset: 'monthly', weekdays: [], monthday: null, by: 'nth', nth: -1, nthDay: 5 };
    expect(recurrenceFromEditor(monthly)).toEqual({ freq: 'monthly', interval: 1, weekdays: [5], monthday: null, nth: -1 });
    const yearly = { preset: 'yearly', weekdays: [], monthday: 15, by: 'date', month: 3 };
    expect(recurrenceFromEditor(yearly)).toEqual({ freq: 'yearly', interval: 1, weekdays: [], monthday: 15, month: 3 });
  });

  it('takes the weekday and month from the start date when the user left them out', () => {
    const editor = { preset: 'yearly', by: 'nth', nth: 1, nthDay: null, month: null, weekdays: [], monthday: null };
    expect(recurrenceFromEditor(editor, '2026-12-02')).toMatchObject({ weekdays: [3], month: 12, nth: 1 });
  });

  it('keeps the plain yearly rule plain', () => {
    expect(recurrenceFromEditor({ preset: 'yearly', weekdays: [], monthday: null })).toEqual({ freq: 'yearly', interval: 1, weekdays: [], monthday: null });
  });
});

describe('recurrenceSummary', () => {
  it('speaks like a person', () => {
    expect(recurrenceSummary(third, t, 'ru')).toBe('каждую 3-ю субботу сентября');
    expect(recurrenceSummary({ freq: 'weekly', interval: 1, weekdays: [2] }, t, 'ru')).toBe('каждый вторник');
    expect(recurrenceSummary({ freq: 'monthly', interval: 1, weekdays: [7], nth: -1 }, t, 'ru')).toBe('каждое последнее воскресенье месяца');
    expect(recurrenceSummary({ freq: 'weekly', interval: 1, weekdays: [1, 2, 3, 4, 5] }, t, 'ru')).toBe('по будням');
    expect(recurrenceSummary({ freq: 'yearly', interval: 1, weekdays: [], monthday: 15, month: 3 }, t, 'ru')).toBe('каждый год, 15 марта');
    expect(recurrenceSummary(null, t, 'ru')).toBe('');
  });
});

describe('routine schedule fields', () => {
  const rule = { freq: 'daily', interval: 1, weekdays: [], monthday: null };

  it('sends a timed routine with its rule and start date and clears the other kinds', () => {
    expect(scheduleFields({ kind: 'at', at: '09:00', recurrence: third, startOn: '2026-12-01' })).toEqual({
      at: '09:00', recurrence: third, startOn: '2026-12-01', onceAt: 0, days: [],
    });
  });

  it('sends a one-time routine as a moment in ms', () => {
    const f = scheduleFields({ kind: 'once', date: '2026-12-02', at: '10:00', recurrence: rule });
    expect(f.onceAt).toBe(new Date('2026-12-02T10:00:00').getTime());
    expect(f).toMatchObject({ at: '', recurrence: null, startOn: '' });
  });

  it('leaves the interval routine without time rules', () => {
    expect(scheduleFields({ kind: 'every', at: '09:00' })).toEqual({ at: '', recurrence: null, startOn: '', onceAt: 0 });
  });

  it('asks for what is missing', () => {
    expect(scheduleProblem({ kind: 'at', at: '' })).toBe('time');
    expect(scheduleProblem({ kind: 'at', at: '09:00' })).toBe('');
    expect(scheduleProblem({ kind: 'once', date: '', at: '10:00' })).toBe('time');
    expect(scheduleProblem({ kind: 'once', date: '2026-12-02', at: '10:00' }, new Date('2026-12-03T00:00:00').getTime())).toBe('past');
    expect(scheduleProblem({ kind: 'once', date: '2026-12-02', at: '10:00' }, new Date('2026-12-01T00:00:00').getTime())).toBe('');
    expect(scheduleProblem({ kind: 'every' })).toBe('');
    expect(onceMoment('', '10:00')).toBe(0);
  });
});

describe('routine card text', () => {
  const fmt = (iso) => iso;
  it('shows time, rule and start date', () => {
    expect(timedScheduleText({ at: '09:30', recurrence: third, startOn: '2026-12-01' }, t, 'ru', fmt)).toBe('в 09:30, каждую 3-ю субботу сентября, с 2026-12-01');
  });
  it('shows a one-time routine and reads it back', () => {
    const onceAt = new Date('2026-12-02T10:00:00').getTime();
    expect(timedScheduleText({ onceAt }, t, 'ru', fmt)).toBe('один раз, 2026-12-02 в 10:00');
    expect(scheduleFromRoutine({ onceAt })).toMatchObject({ kind: 'once', date: '2026-12-02', at: '10:00' });
  });
  it('leaves interval routines to the screen', () => {
    expect(timedScheduleText({ at: '', everyMinutes: 60 }, t, 'ru', fmt)).toBe('');
    expect(scheduleFromRoutine({ at: '', everyMinutes: 60 }).kind).toBe('every');
  });
});
