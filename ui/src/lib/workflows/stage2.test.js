import { beforeEach, describe, expect, it } from 'vitest';
import { CanvasModel } from './canvasModel.svelte.js';
import { SHAPE_CHOICES, canConnect, edgeFlow, isEnabled, kindsBetween, kindsFor, makeNode, outTypeOf, starterGraph, toFlow, toGraph, typeProblem, validateGraph } from './graph.js';
import { hitsStroke, limitPoints, pathOf, thin, MAX_POINTS } from './strokes.js';

describe('stage 2 kinds', () => {
  it('llm and condition are real nodes now', () => {
    expect(isEnabled('llm')).toBe(true);
    expect(isEnabled('condition')).toBe(true);
    expect(kindsFor('bot', 'source')).toEqual(expect.arrayContaining(['llm', 'condition', 'approval', 'output']));
  });
  it('kindsBetween needs both an input and an output', () => {
    const k = kindsBetween('trigger', 'output');
    expect(k).toEqual(expect.arrayContaining(['bot', 'llm', 'condition', 'approval']));
    expect(k).not.toContain('output');
    expect(k).not.toContain('prompt');
    expect(k).not.toContain('shape');
  });
  it('shape keeps its size and fields through the flow round trip', () => {
    const g = { nodes: [{ id: 's', kind: 'shape', x: 5, y: 6, w: 150, h: 90, fields: { shape: 'star', fill: 'mint', text: 'x' } }], edges: [], strokes: [] };
    const { nodes, edges } = toFlow(g);
    expect(nodes[0].type).toBe('wfshape');
    expect(toGraph(nodes, edges)).toEqual(g);
  });
  it('a condition edge with a branch validates, without one does not', () => {
    const g = { nodes: [makeNode('condition', 0, 0), makeNode('output', 400, 0)], edges: [], strokes: [] };
    g.edges = [{ id: 'e', from: g.nodes[0].id, to: g.nodes[1].id, branch: 'yes' }];
    expect(validateGraph(g)).toEqual([]);
    delete g.edges[0].branch;
    expect(validateGraph(g).map((p) => p.code)).toContain('missing_branch');
  });
  it('the branch not taken is marked off', () => {
    const steps = new Map([['c', { status: 'done', branch: 'no' }]]);
    expect(edgeFlow({ source: 'c', target: 'a', branch: 'yes' }, steps)).toBe('off');
    expect(edgeFlow({ source: 'c', target: 'b', branch: 'no' }, steps)).toBe('pending');
  });
});

describe('strokes', () => {
  it('thins nearby points and rounds', () => {
    expect(thin([[0, 0], [0.4, 0.4], [3, 0], [3.2, 0.1], [9.123, 0]], 1.5)).toEqual([[0, 0], [3, 0], [9.1, 0]]);
  });
  it('keeps at most MAX_POINTS with both ends', () => {
    const pts = Array.from({ length: 5000 }, (_, i) => [i, i]);
    const out = limitPoints(pts);
    expect(out).toHaveLength(MAX_POINTS);
    expect(out[0]).toEqual([0, 0]);
    expect(out[out.length - 1]).toEqual([4999, 4999]);
  });
  it('builds a path for one, two and many points', () => {
    expect(pathOf([])).toBe('');
    expect(pathOf([[1, 2]])).toContain('M1 2');
    expect(pathOf([[0, 0], [5, 5]])).toBe('M0 0L5 5');
    expect(pathOf([[0, 0], [5, 5], [10, 0]])).toMatch(/^M0 0Q5 5 7\.5 2\.5L10 0$/);
  });
  it('eraser hits near the line only', () => {
    const s = { width: 4, points: [[0, 0], [100, 0]] };
    expect(hitsStroke(s, 50, 8, 10)).toBe(true);
    expect(hitsStroke(s, 50, 30, 10)).toBe(false);
    expect(hitsStroke({ width: 2, points: [[10, 10]] }, 12, 10, 3)).toBe(true);
  });
});

describe('CanvasModel stage 2', () => {
  let m;
  let changes;
  beforeEach(() => {
    changes = 0;
    m = new CanvasModel({ onchange: () => (changes += 1) });
    m.load(starterGraph());
  });

  it('strokes are saved, undone and erased', () => {
    expect(m.addStroke({ id: 's1', brush: 'pen', color: 'ink', width: 3, points: [[0, 0], [5, 5]] })).toBe(true);
    expect(m.getGraph().strokes).toHaveLength(1);
    expect(changes).toBe(1);
    m.undo();
    expect(m.strokes).toHaveLength(0);
    m.redo();
    expect(m.strokes).toHaveLength(1);
    m.removeStrokes(['s1']);
    expect(m.strokes).toHaveLength(0);
    expect(m.canUndo).toBe(true);
  });

  it('refuses the 501st stroke', () => {
    for (let i = 0; i < 500; i += 1) m.strokes = [...m.strokes, { id: 's' + i, brush: 'pen', color: 'ink', width: 3, points: [[0, 0]] }];
    expect(m.addStroke({ id: 'x', brush: 'pen', color: 'ink', width: 3, points: [[0, 0]] })).toBe(false);
  });

  it('copy, cut and paste keep inner edges and place at a point', () => {
    m.selectAll();
    expect(m.copy()).toBe(true);
    m.clearSelection();
    const ids = m.paste({ x: 1000, y: 500 });
    expect(ids).toHaveLength(3);
    expect(m.nodes).toHaveLength(6);
    expect(m.edges).toHaveLength(4);
    const pasted = m.nodes.filter((n) => ids.includes(n.id));
    expect(Math.min(...pasted.map((n) => n.position.x))).toBe(1000);
    expect(Math.min(...pasted.map((n) => n.position.y))).toBe(500);
    m.clearSelection();
    m.nodes = m.nodes.map((n) => (ids.includes(n.id) ? { ...n, selected: true } : n));
    m.cut();
    expect(m.nodes).toHaveLength(3);
    expect(m.paste()).toHaveLength(3);
  });

  it('paste without a clipboard does nothing', () => {
    expect(m.paste()).toEqual([]);
  });

  it('inserts a node into an edge as one undo step', () => {
    const edge = m.edges[0];
    const node = m.insertIntoEdge(edge.id, 'approval');
    expect(node).toBeTruthy();
    expect(m.nodes).toHaveLength(4);
    expect(m.edges).toHaveLength(3);
    expect(m.edges.some((e) => e.source === edge.source && e.target === node.id)).toBe(true);
    expect(m.edges.some((e) => e.source === node.id && e.target === edge.target)).toBe(true);
    m.undo();
    expect(m.nodes).toHaveLength(3);
    expect(m.edges).toHaveLength(2);
  });

  it('refuses an insert that would break the port rules', () => {
    expect(m.insertIntoEdge(m.edges[0].id, 'trigger')).toBeNull();
    expect(m.insertIntoEdge(m.edges[0].id, 'note')).toBeNull();
  });

  it('inserting a condition gives the outgoing edge a branch', () => {
    const edge = m.edges[0];
    const node = m.insertIntoEdge(edge.id, 'condition');
    const out = m.edges.find((e) => e.source === node.id);
    expect(out.data.branch).toBe('yes');
    expect(validateGraph(m.getGraph())).toEqual([]);
  });

  it('brings to front and sends to back', () => {
    const first = m.nodes[0].id;
    m.bringToFront(first);
    expect(m.nodes.at(-1).id).toBe(first);
    m.sendToBack(first);
    expect(m.nodes[0].id).toBe(first);
  });

  it('resets the size of a note', () => {
    const n = m.addNode('note', 0, 0);
    m.setSize(n.id, 500, 400);
    m.resetSize(n.id);
    const back = m.nodes.find((x) => x.id === n.id);
    expect([back.width, back.height]).toEqual([220, 160]);
  });

  it('Tab order goes left to right and wraps, backwards from the start', () => {
    const order = [...m.nodes].sort((a, b) => a.position.x - b.position.x).map((n) => n.id);
    expect(m.neighbour(1).id).toBe(order[0]);
    m.nodes = m.nodes.map((n) => (n.id === order[0] ? { ...n, selected: true } : n));
    expect(m.neighbour(1).id).toBe(order[1]);
    expect(m.neighbour(-1).id).toBe(order[2]);
    m.clearSelection();
    expect(m.neighbour(-1).id).toBe(order[2]);
  });

  it('locking a node survives the round trip', () => {
    m.toggleLock(m.nodes[0].id);
    const g = m.getGraph();
    expect(g.nodes[0].locked).toBe(true);
    const flow = toFlow(g);
    expect(flow.nodes[0].draggable).toBe(false);
  });
});

describe('port types', () => {
  const mk = (kind, fields = {}) => toFlow({ nodes: [{ id: kind + JSON.stringify(fields).length, kind, x: 0, y: 0, fields }], edges: [] }).nodes[0];

  it('knows what each node puts out', () => {
    expect(outTypeOf('trigger')).toBe('text');
    expect(outTypeOf('image')).toBe('image');
    expect(outTypeOf('asset', { name: 'a.jpg', mime: 'image/jpeg', path: 'p' })).toBe('image');
    expect(outTypeOf('asset', { name: 'a.pdf', mime: 'application/pdf', path: 'p' })).toBe('file');
    expect(outTypeOf('asset', { text: 'hello' })).toBe('text');
    expect(outTypeOf('approval')).toBe('any');
    expect(outTypeOf('output')).toBeNull();
  });

  it('accepts matching types and refuses the rest', () => {
    expect(typeProblem('asset', { name: 'a.pdf', path: 'p', mime: 'application/pdf' }, 'llm')).toBeNull();
    expect(typeProblem('image', {}, 'llm')).toBeNull();
    expect(typeProblem('image', {}, 'condition')).toBe('type');
    expect(typeProblem('asset', { name: 'a.pdf', path: 'p', mime: 'application/pdf' }, 'condition')).toBe('type');
    expect(typeProblem('image', {}, 'output')).toBeNull();
    expect(typeProblem('image', {}, 'approval')).toBeNull();
  });

  it('canConnect uses the types of the real nodes', () => {
    const a = mk('asset', { name: 'a.pdf', mime: 'application/pdf', path: 'p' });
    const c = mk('condition');
    const l = mk('llm');
    expect(canConnect([a, c], [], { source: a.id, target: c.id, sourceHandle: 'out' })).toBe(false);
    expect(canConnect([a, l], [], { source: a.id, target: l.id, sourceHandle: 'out' })).toBe(true);
  });

  it('the model reports a type mismatch as its own problem', () => {
    const m = new CanvasModel({});
    m.load({
      nodes: [
        { id: 'i', kind: 'image', x: 0, y: 0, fields: {} },
        { id: 'c', kind: 'condition', x: 400, y: 0, fields: {} },
      ],
      edges: [],
      strokes: [],
    });
    expect(m.connect({ source: 'i', target: 'c', sourceHandle: 'out' }).problem).toBe('type');
  });

  it('offers only fitting kinds after dragging from an image output', () => {
    const k = kindsFor('image', 'source', {});
    expect(k).toEqual(expect.arrayContaining(['llm', 'image', 'bot', 'approval', 'output']));
    expect(k).not.toContain('condition');
  });

  it('inverted triangle is a flipped triangle for the backend', () => {
    const d = SHAPE_CHOICES.find((c) => c.id === 'triangle_down');
    expect(d.shape).toBe('triangle');
    expect(d.flip).toBe(true);
    expect(SHAPE_CHOICES.map((c) => c.id)).toHaveLength(7);
  });

  it('asset and image get default fields', () => {
    expect(makeNode('asset', 0, 0).fields).toEqual({ name: '', path: '', mime: '', size: 0 });
    expect(makeNode('image', 0, 0).fields.size).toBe('1024x1024');
  });
});
