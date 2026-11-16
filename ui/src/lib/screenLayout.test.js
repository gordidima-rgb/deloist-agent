import { describe, it, expect } from 'vitest';
import {
  clampPanel, widthFromPointer, widthByKey, nearestCorner, validCorner, nextMode, badge, loadLayout, saveLayout,
  MIN_PANEL, MIN_CHAT, DEFAULT_PANEL, KEY,
} from './screenLayout.js';

describe('clampPanel', () => {
  it('keeps both sides at a sane minimum', () => {
    expect(clampPanel(100, 1200)).toBe(MIN_PANEL);
    expect(clampPanel(5000, 1200)).toBe(1200 - MIN_CHAT - 36);
    expect(clampPanel(600, 1200)).toBe(600);
  });
  it('lets the chat give way in a narrow window', () => {
    expect(clampPanel(900, 700)).toBe(MIN_PANEL);
  });
  it('falls back to the default for garbage', () => {
    expect(clampPanel(NaN, 1400)).toBe(DEFAULT_PANEL);
  });
});

describe('splitter', () => {
  it('turns the pointer position into a width', () => {
    expect(widthFromPointer(700, 1280, 1200)).toBe(568);
    expect(widthFromPointer(10, 1280, 1200)).toBe(1200 - MIN_CHAT - 36);
  });
  it('moves with the arrows, Shift takes a big step', () => {
    expect(widthByKey(500, 'ArrowLeft', false, 1200)).toBe(524);
    expect(widthByKey(500, 'ArrowRight', true, 1200)).toBe(404);
    expect(widthByKey(500, 'Home', false, 1200)).toBe(MIN_PANEL);
    expect(widthByKey(500, 'End', false, 1200)).toBe(1200 - MIN_CHAT - 36);
    expect(widthByKey(500, 'a', false, 1200)).toBe(500);
  });
});

describe('island corners', () => {
  const bounds = { left: 300, top: 0, width: 1000, height: 800 };
  it('picks the nearest corner by the island centre', () => {
    expect(nearestCorner({ left: 320, top: 20, width: 300, height: 300 }, bounds)).toBe('tl');
    expect(nearestCorner({ left: 900, top: 20, width: 300, height: 300 }, bounds)).toBe('tr');
    expect(nearestCorner({ left: 320, top: 480, width: 300, height: 300 }, bounds)).toBe('bl');
    expect(nearestCorner({ left: 900, top: 480, width: 300, height: 300 }, bounds)).toBe('br');
  });
  it('rejects unknown corners', () => {
    expect(validCorner('tl')).toBe('tl');
    expect(validCorner('x')).toBe('br');
  });
});

describe('modes', () => {
  const split = { mode: 'split', before: 'split' };
  it('maximizes and goes back with Esc', () => {
    const max = nextMode(split, 'maximize');
    expect(max.mode).toBe('max');
    expect(nextMode(max, 'back').mode).toBe('split');
    expect(nextMode(split, 'back')).toBe(split);
    expect(nextMode(max, 'maximize').mode).toBe('split');
  });
  it('restores from the pill into the mode it left', () => {
    const min = nextMode(nextMode(split, 'maximize'), 'minimize');
    expect(min.mode).toBe('min');
    expect(nextMode(min, 'restore').mode).toBe('max');
    expect(nextMode(nextMode(split, 'minimize'), 'restore').mode).toBe('split');
    expect(nextMode(split, 'restore')).toBe(split);
  });
});

describe('badge', () => {
  it('is empty at zero and capped at 9+', () => {
    expect(badge(0)).toBe('');
    expect(badge(3)).toBe('3');
    expect(badge(12)).toBe('9+');
  });
});

describe('storage', () => {
  const memory = () => {
    const data = new Map();
    return { getItem: (k) => data.get(k) ?? null, setItem: (k, v) => data.set(k, v) };
  };
  it('round-trips the layout', () => {
    const s = memory();
    saveLayout(s, { width: 480, corner: 'tl', mode: 'max' });
    expect(loadLayout(s)).toEqual({ width: 480, corner: 'tl', mode: 'max' });
  });
  it('survives a broken or blocked storage', () => {
    const s = memory();
    s.setItem(KEY, '{oops');
    expect(loadLayout(s).width).toBe(DEFAULT_PANEL);
    const blocked = { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); } };
    expect(loadLayout(blocked).corner).toBe('br');
    expect(() => saveLayout(blocked, { width: 1, corner: 'br', mode: 'split' })).not.toThrow();
    expect(loadLayout(undefined).mode).toBe('split');
  });
});
