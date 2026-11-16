import { describe, expect, it } from 'vitest';
import {
  canConnect,
  cloneSelection,
  connectionProblem,
  edgeFlow,
  graphKey,
  isRunOpen,
  kindsFor,
  latestSteps,
  makeNode,
  normalizeSchedule,
  scheduleDefaults,
  starterGraph,
  stepSeconds,
  toFlow,
  toGraph,
  toMs,
  validateGraph,
} from './graph.js';

const sample = () => ({
  nodes: [
    { id: 't', kind: 'trigger', x: 0, y: 0, fields: { trigger: 'manual' } },
    { id: 'p', kind: 'prompt', x: 0, y: 200, fields: { text: 'hi' } },
    { id: 'b', kind: 'bot', x: 400, y: 0, fields: { bot_id: 'a', task: 'x' } },
    { id: 'o', kind: 'output', x: 800, y: 0, fields: { target: 'notify' } },
    { id: 'n', kind: 'note', x: 0, y: -200, w: 200, h: 120, fields: { text: 'n', color: 'mint' } },
  ],
  edges: [
    { id: 'e1', from: 't', to: 'b' },
    { id: 'e2', from: 'b', to: 'o' },
  ],
  strokes: [],
});

describe('connectionProblem', () => {
  it('rejects input into trigger and prompt', () => {
    expect(connectionProblem('bot', 'trigger')).toBe('no_input');
    expect(connectionProblem('bot', 'prompt')).toBe('no_input');
  });
  it('rejects output from output', () => {
    expect(connectionProblem('output', 'bot')).toBe('no_output');
  });
  it('rejects annotations on either side', () => {
    expect(connectionProblem('note', 'bot')).toBe('annotation');
    expect(connectionProblem('bot', 'text')).toBe('annotation');
  });
  it('allows the normal chain', () => {
    expect(connectionProblem('trigger', 'bot')).toBeNull();
    expect(connectionProblem('prompt', 'approval')).toBeNull();
    expect(connectionProblem('approval', 'output')).toBeNull();
  });
});

describe('canConnect', () => {
  const { nodes, edges } = toFlow(sample());
  it('refuses self loops, duplicates and unknown nodes', () => {
    expect(canConnect(nodes, edges, { source: 'b', target: 'b' })).toBe(false);
    expect(canConnect(nodes, edges, { source: 't', target: 'b', sourceHandle: 'out' })).toBe(false);
    expect(canConnect(nodes, edges, { source: 't', target: 'zzz' })).toBe(false);
  });
  it('accepts a new valid edge, refuses an invalid one', () => {
    expect(canConnect(nodes, edges, { source: 'p', target: 'b', sourceHandle: 'out' })).toBe(true);
    expect(canConnect(nodes, edges, { source: 'o', target: 'b' })).toBe(false);
    expect(canConnect(nodes, edges, { source: 'n', target: 'b' })).toBe(false);
  });
  it('requires a branch from a condition', () => {
    const cond = [...nodes, ...toFlow({ nodes: [{ id: 'c', kind: 'condition', x: 0, y: 0, fields: {} }], edges: [] }).nodes];
    expect(canConnect(cond, edges, { source: 'c', target: 'o', sourceHandle: 'out' })).toBe(false);
    expect(canConnect(cond, edges, { source: 'c', target: 'o', sourceHandle: 'yes' })).toBe(true);
  });
});

describe('kindsFor', () => {
  it('after dragging from an output only kinds with an input are offered', () => {
    const k = kindsFor('bot', 'source');
    expect(k).toContain('approval');
    expect(k).toContain('output');
    expect(k).not.toContain('trigger');
    expect(k).not.toContain('note');
    expect(k).toContain('llm');
  });
  it('after dragging from an input only kinds with an output are offered', () => {
    const k = kindsFor('bot', 'target');
    expect(k).toContain('trigger');
    expect(k).toContain('prompt');
    expect(k).not.toContain('output');
  });
});

describe('graph <-> flow', () => {
  it('round-trips without losing fields', () => {
    const g = sample();
    const { nodes, edges } = toFlow(g);
    expect(toGraph(nodes, edges)).toEqual(g);
  });
  it('keeps the branch of a condition edge', () => {
    const g = { nodes: [{ id: 'c', kind: 'condition', x: 1, y: 2, fields: {} }, { id: 'o', kind: 'output', x: 5, y: 5, fields: {} }], edges: [{ id: 'e', from: 'c', to: 'o', branch: 'no' }], strokes: [] };
    const { nodes, edges } = toFlow(g);
    expect(edges[0].sourceHandle).toBe('no');
    expect(toGraph(nodes, edges).edges[0].branch).toBe('no');
  });
  it('graphKey ignores runtime-only properties', () => {
    const { nodes, edges } = toFlow(sample());
    const a = graphKey(toGraph(nodes, edges));
    const b = graphKey(toGraph(nodes.map((n) => ({ ...n, selected: true })), edges));
    expect(a).toBe(b);
  });
});

describe('validateGraph', () => {
  it('passes a clean graph', () => {
    expect(validateGraph(sample())).toEqual([]);
  });
  it('finds dangling, duplicate and wrong-kind edges', () => {
    const g = sample();
    g.edges.push({ id: 'e3', from: 't', to: 'b' }, { id: 'e4', from: 'o', to: 'b' }, { id: 'e5', from: 'b', to: 'nope' }, { id: 'e6', from: 'n', to: 'b' });
    const codes = validateGraph(g).map((p) => `${p.code}:${p.edge_id}`);
    expect(codes).toContain('duplicate_edge:e3');
    expect(codes).toContain('bad_edge:e4');
    expect(codes).toContain('dangling_edge:e5');
    expect(codes).toContain('bad_edge:e6');
  });
  it('requires branches only on conditions', () => {
    const g = { nodes: [{ id: 'c', kind: 'condition', x: 0, y: 0, fields: {} }, { id: 'o', kind: 'output', x: 0, y: 0, fields: {} }, { id: 'b', kind: 'bot', x: 0, y: 0, fields: {} }], edges: [{ id: 'a', from: 'c', to: 'o' }, { id: 'b', from: 'b', to: 'o', branch: 'yes' }], strokes: [] };
    const codes = validateGraph(g).map((p) => p.code);
    expect(codes).toContain('missing_branch');
    expect(codes).toContain('unexpected_branch');
  });
  it('starter graph is valid', () => {
    expect(validateGraph(starterGraph())).toEqual([]);
  });
});

describe('cloneSelection', () => {
  it('copies chosen nodes and edges between them with new ids', () => {
    const { nodes, edges } = toFlow(sample());
    const c = cloneSelection(nodes, edges, ['t', 'b']);
    expect(c.nodes).toHaveLength(2);
    expect(c.edges).toHaveLength(1);
    const ids = new Set(nodes.map((n) => n.id));
    for (const n of c.nodes) expect(ids.has(n.id)).toBe(false);
    expect(c.nodes[0].position.x).toBe(40);
    expect(new Set(c.nodes.map((n) => n.id)).has(c.edges[0].source)).toBe(true);
  });
  it('does not share field objects with the original', () => {
    const { nodes, edges } = toFlow(sample());
    const c = cloneSelection(nodes, edges, ['p']);
    c.nodes[0].data.fields.text = 'changed';
    expect(nodes.find((n) => n.id === 'p').data.fields.text).toBe('hi');
  });
});

describe('makeNode', () => {
  it('gives every kind sensible default fields', () => {
    expect(makeNode('trigger', 1.4, 2.6, {}).fields).toEqual({ trigger: 'manual' });
    expect(makeNode('bot', 0, 0).fields).toEqual({ bot_id: '', task: '' });
    expect(makeNode('note', 0, 0).w).toBe(220);
    expect(makeNode('trigger', 1.4, 2.6).x).toBe(1);
  });
});

describe('run helpers', () => {
  it('takes the latest attempt of each node', () => {
    const run = { steps: [{ node_id: 'a', attempt: 1, status: 'failed' }, { node_id: 'a', attempt: 2, status: 'done' }, { node_id: 'b', attempt: 1, status: 'running' }] };
    const m = latestSteps(run);
    expect(m.get('a').status).toBe('done');
    expect(m.get('b').status).toBe('running');
    expect(latestSteps(null).size).toBe(0);
  });
  it('reads seconds, milliseconds and iso timestamps', () => {
    expect(toMs(1700000000)).toBe(1700000000000);
    expect(toMs(1700000000000)).toBe(1700000000000);
    expect(toMs('2026-11-16T10:00:00Z')).toBe(Date.parse('2026-11-16T10:00:00Z'));
    expect(toMs(null)).toBe(0);
  });
  it('counts step seconds, running steps against now', () => {
    expect(stepSeconds({ started_at: 100, ended_at: 104.5 })).toBe(4.5);
    expect(stepSeconds({ started_at: 100 }, 103000)).toBe(3);
  });
  it('knows which runs are open', () => {
    expect(isRunOpen({ status: 'approval' })).toBe(true);
    expect(isRunOpen({ status: 'done' })).toBe(false);
    expect(isRunOpen(null)).toBe(false);
  });
  it('flows along an edge only while the target runs', () => {
    const steps = new Map([
      ['a', { status: 'done' }],
      ['b', { status: 'running' }],
    ]);
    expect(edgeFlow({ source: 'a', target: 'b' }, steps)).toBe('live');
    expect(edgeFlow({ source: 'a', target: 'c' }, steps)).toBe('pending');
    expect(edgeFlow({ source: 'x', target: 'y' }, steps)).toBe('idle');
  });
});

describe('schedule', () => {
  it('has defaults for every kind', () => {
    expect(scheduleDefaults('weekly')).toEqual({ kind: 'weekly', time: '09:00', weekday: 1 });
    expect(scheduleDefaults('interval').unit).toBe('minutes');
    expect(scheduleDefaults('once').date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
  it('keeps an interval at three minutes or more', () => {
    expect(normalizeSchedule({ kind: 'interval', amount: 1, unit: 'minutes' }).amount).toBe(3);
    expect(normalizeSchedule({ kind: 'interval', amount: 1, unit: 'hours' }).amount).toBe(1);
  });
  it('clamps minute, day and weekday', () => {
    expect(normalizeSchedule({ kind: 'hourly', minute: 99 }).minute).toBe(59);
    expect(normalizeSchedule({ kind: 'monthly', day: 40 }).day).toBe(31);
    expect(normalizeSchedule({ kind: 'weekly', weekday: 0 }).weekday).toBe(1);
  });
});
