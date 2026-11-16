import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CanvasModel } from './canvasModel.svelte.js';
import { History } from './history.js';
import { starterGraph } from './graph.js';

describe('History', () => {
  it('undoes and redoes, and a new push clears redo', () => {
    const h = new History(100);
    h.reset('a');
    expect(h.push('b')).toBe(true);
    expect(h.push('b')).toBe(false);
    h.push('c');
    expect(h.undo()).toBe('b');
    expect(h.undo()).toBe('a');
    expect(h.undo()).toBeNull();
    expect(h.redo()).toBe('b');
    h.push('x');
    expect(h.canRedo).toBe(false);
  });
  it('keeps at most `limit` steps', () => {
    const h = new History(100);
    h.reset('0');
    for (let i = 1; i <= 130; i += 1) h.push(String(i));
    let n = 0;
    while (h.undo() !== null) n += 1;
    expect(n).toBe(100);
  });
});

describe('CanvasModel', () => {
  let changes;
  let m;
  beforeEach(() => {
    vi.useFakeTimers();
    changes = 0;
    m = new CanvasModel({ onchange: () => (changes += 1) });
    m.load(starterGraph());
  });
  afterEach(() => vi.useRealTimers());

  it('loads a graph without raising a change', () => {
    expect(m.nodes).toHaveLength(3);
    expect(m.edges).toHaveLength(2);
    expect(changes).toBe(0);
    expect(m.canUndo).toBe(false);
  });

  it('add, undo, redo restore the node list', () => {
    m.addNode('prompt', 10, 20);
    expect(m.nodes).toHaveLength(4);
    expect(changes).toBe(1);
    expect(m.canUndo).toBe(true);
    m.undo();
    expect(m.nodes).toHaveLength(3);
    expect(m.canRedo).toBe(true);
    m.redo();
    expect(m.nodes).toHaveLength(4);
    expect(m.nodes.find((n) => n.data.kind === 'prompt').position).toEqual({ x: 10, y: 20 });
  });

  it('typing is one undo step after a pause', () => {
    const id = m.nodes.find((n) => n.data.kind === 'bot').id;
    m.setField(id, 'task', 'a');
    m.setField(id, 'task', 'ab');
    m.setField(id, 'task', 'abc');
    expect(changes).toBe(0);
    vi.advanceTimersByTime(700);
    expect(changes).toBe(1);
    m.undo();
    expect(m.nodes.find((n) => n.id === id).data.fields.task).toBe('');
  });

  it('undo flushes a half-typed edit instead of losing it', () => {
    const id = m.nodes.find((n) => n.data.kind === 'bot').id;
    m.setField(id, 'task', 'draft');
    m.undo();
    expect(m.nodes.find((n) => n.id === id).data.fields.task).toBe('');
    m.redo();
    expect(m.nodes.find((n) => n.id === id).data.fields.task).toBe('draft');
  });

  it('removing a node removes its edges and can be undone', () => {
    const bot = m.nodes.find((n) => n.data.kind === 'bot');
    m.nodes = m.nodes.map((n) => (n.id === bot.id ? { ...n, selected: true } : n));
    expect(m.removeSelected()).toBe(true);
    expect(m.nodes).toHaveLength(2);
    expect(m.edges).toHaveLength(0);
    m.undo();
    expect(m.nodes).toHaveLength(3);
    expect(m.edges).toHaveLength(2);
  });

  it('duplicates the selection with inner edges only', () => {
    m.selectAll();
    const ids = m.duplicateSelected();
    expect(ids).toHaveLength(3);
    expect(m.nodes).toHaveLength(6);
    expect(m.edges).toHaveLength(4);
  });

  it('connects valid ports and explains refusals', () => {
    const p = m.addNode('prompt', 0, 300);
    const bot = m.nodes.find((n) => n.data.kind === 'bot');
    const ok = m.connect({ source: p.id, target: bot.id, sourceHandle: 'out' });
    expect(ok.edge).toBeTruthy();
    m.addEdge(ok.edge);
    expect(m.edges).toHaveLength(3);
    expect(m.connect({ source: p.id, target: bot.id, sourceHandle: 'out' }).problem).toBe('duplicate');
    const trig = m.nodes.find((n) => n.data.kind === 'trigger');
    expect(m.connect({ source: bot.id, target: trig.id }).problem).toBe('no_input');
    const out = m.nodes.find((n) => n.data.kind === 'output');
    expect(m.connect({ source: out.id, target: bot.id }).problem).toBe('no_output');
    expect(m.connect({ source: bot.id, target: bot.id }).problem).toBe('self');
  });

  it('search finds nodes by kind or text', () => {
    const id = m.nodes.find((n) => n.data.kind === 'bot').id;
    m.setField(id, 'task', 'Проверь цены');
    expect(m.search('цены').map((n) => n.id)).toEqual([id]);
    expect(m.search('')).toEqual([]);
  });

  it('a graph key that does not change raises no change', () => {
    m.commit();
    expect(changes).toBe(0);
  });
});
