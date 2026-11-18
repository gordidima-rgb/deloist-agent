import { describe, expect, it } from 'vitest';
import { awayTasks, filterTasks, leftAt } from './tasks.js';

const DAY = 24 * 3600 * 1000;
const now = 100 * DAY;
const t = (id, state, created, finished = 0) => ({ id, state, created, finished });

describe('filterTasks', () => {
  const list = [t('a', 'running', now - 10 * DAY), t('b', 'done', now - 2 * DAY, now - DAY), t('c', 'done', now - 9 * DAY, now - 8 * DAY), t('d', 'queued', now - 1000)];
  it('open keeps only unfinished, newest first', () => {
    expect(filterTasks(list, 'open', now).map((x) => x.id)).toEqual(['d', 'a']);
  });
  it('week keeps open ones and those touched in 7 days', () => {
    expect(filterTasks(list, 'week', now).map((x) => x.id)).toEqual(['d', 'b', 'a']);
  });
});

describe('awayTasks', () => {
  const list = [t('a', 'running', 1, 0), t('b', 'done', 1, now - 3 * 3600000), t('c', 'error', 1, now - 9 * DAY), t('d', 'interrupted', 1, now - 60000)];
  it('never seen: last 7 days of closed tasks', () => {
    expect(awayTasks(list, undefined, now).map((x) => x.id)).toEqual(['d', 'b']);
  });
  it('after the window was left', () => {
    expect(awayTasks(list, now - 3600000, now).map((x) => x.id)).toEqual(['d']);
  });
  it('dismissed (0) shows nothing', () => {
    expect(awayTasks(list, 0, now)).toEqual([]);
  });
});

describe('leftAt', () => {
  it('starts a new away period only after the previous one was dismissed', () => {
    expect(leftAt(0, 500)).toBe(500);
    expect(leftAt(300, 500)).toBe(300);
    expect(leftAt(undefined, 500)).toBeUndefined();
  });
});
