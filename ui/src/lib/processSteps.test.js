import { describe, expect, it } from 'vitest';
import { orderSteps } from './processSteps.js';

const n = (id, kind, x = 0, y = 0) => ({ id, kind, x, y, fields: {} });

describe('orderSteps', () => {
  it('follows the lines and skips annotations', () => {
    const graph = {
      nodes: [n('b', 'bot', 300), n('note', 'note', 0, 90), n('t', 'trigger', 900), n('o', 'output', 100)],
      edges: [
        { id: 'e1', from: 't', to: 'b' },
        { id: 'e2', from: 'b', to: 'o' },
      ],
    };
    expect(orderSteps(graph).map((x) => x.id)).toEqual(['t', 'b', 'o']);
  });

  it('keeps steps that no line reaches, left to right', () => {
    const graph = { nodes: [n('b', 'bot', 200), n('a', 'prompt', 100)], edges: [] };
    expect(orderSteps(graph).map((x) => x.id)).toEqual(['a', 'b']);
    expect(orderSteps(null)).toEqual([]);
  });

  it('does not drop steps of a cycle', () => {
    const graph = { nodes: [n('a', 'bot', 0), n('b', 'bot', 10)], edges: [{ id: 'e1', from: 'a', to: 'b' }, { id: 'e2', from: 'b', to: 'a' }] };
    expect(orderSteps(graph).map((x) => x.id)).toEqual(['a', 'b']);
  });
});
