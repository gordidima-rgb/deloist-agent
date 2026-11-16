import { describe, it, expect } from 'vitest';
import { turnDurations, durationText } from './turnTime.js';

const u = (id, at) => ({ id, at, role: 'user', text: 'q' });
const a = (id, at, extra = {}) => ({ id, at, role: 'assistant', text: 'ответ', ...extra });

describe('turnDurations', () => {
  it('measures from the question to the final answer, skipping work steps', () => {
    const list = [
      { id: 't', at: 1000, role: 'time', text: 'Время' },
      u('u1', 1030),
      a('s1', 61_000, { toolCalls: [{ id: 'c', name: 'x' }] }),
      a('f1', 541_030),
      u('u2', 600_000),
      a('f2', 612_000),
    ];
    const d = turnDurations(list);
    expect(d.get('f1')).toBe(540_030);
    expect(d.get('f2')).toBe(12_000);
    expect(d.has('s1')).toBe(false);
  });

  it('shows nothing without a start or with a sub-second gap', () => {
    expect(turnDurations([a('x', 5000)]).size).toBe(0);
    expect(turnDurations([u('u', 5000), a('x', 5400)]).size).toBe(0);
    expect(turnDurations([u('u', 5000), { id: 'e', at: 9000, role: 'assistant', text: 'e', error: true }]).size).toBe(0);
  });

  it('a follow-up sent mid-turn does not shorten the turn', () => {
    const d = turnDurations([u('u1', 1000), u('u2', 50_000), a('f', 121_000)]);
    expect(d.get('f')).toBe(120_000);
  });
});

describe('durationText', () => {
  it('formats seconds, minutes and hours', () => {
    expect(durationText(8_400)).toBe('8 с');
    expect(durationText(541_000)).toBe('9 мин');
    expect(durationText(3_900_000)).toBe('1 ч 5 мин');
    expect(durationText(7_200_000)).toBe('2 ч');
    expect(durationText(0)).toBe('');
  });
});
