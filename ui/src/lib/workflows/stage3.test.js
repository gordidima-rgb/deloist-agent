import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CanvasModel } from './canvasModel.svelte.js';
import { starterGraph, isLinked, isMessenger, triggerOf, targetOf, TRIGGER_KINDS, OUTPUT_TARGETS } from './graph.js';
import { fileUrlFor, uploadAssetTo, outputParts, badOutputPath } from './assets.js';

describe('assets', () => {
  it('encodes each path segment but keeps the slashes', () => {
    expect(fileUrlFor('wf_1', 'assets/мой файл.png')).toBe('/api/workflows/wf_1/files/assets/%D0%BC%D0%BE%D0%B9%20%D1%84%D0%B0%D0%B9%D0%BB.png');
  });
  it('passes ready urls through', () => {
    expect(fileUrlFor('w', 'data:image/png;base64,AAA')).toBe('data:image/png;base64,AAA');
    expect(fileUrlFor('w', 'blob:x')).toBe('blob:x');
  });
  it('uploads a file as base64 JSON', async () => {
    const api = vi.fn(async () => ({ name: 'a.txt', path: 'assets/a.txt' }));
    const file = new File(['hello'], 'a.txt', { type: 'text/plain' });
    const res = await uploadAssetTo(api, 'wf_1', file);
    expect(res.path).toBe('assets/a.txt');
    const [path, opts] = api.mock.calls[0];
    expect(path).toBe('/api/workflows/wf_1/assets');
    expect(opts.method).toBe('POST');
    expect(opts.body).toEqual({ name: 'a.txt', data: btoa('hello'), mime: 'text/plain' });
  });
  it('refuses files above 20 MB before reading them', async () => {
    const big = { size: 21 * 1024 * 1024, name: 'x.bin', type: '' };
    await expect(uploadAssetTo(vi.fn(), 'w', big)).rejects.toMatchObject({ status: 413 });
  });
  it('checks the output path the way the server does', () => {
    expect(outputParts('отчёты\\итог.txt')).toEqual(['отчёты', 'итог.txt']);
    expect(badOutputPath('a/b.txt')).toBe(false);
    expect(badOutputPath('')).toBe(false);
    expect(badOutputPath('/etc/x')).toBe(true);
    expect(badOutputPath('C:\\x.txt')).toBe(true);
    expect(badOutputPath('a/../b')).toBe(true);
  });
});

describe('kinds', () => {
  it('lists six trigger kinds and six output targets', () => {
    expect(TRIGGER_KINDS.map((k) => k.id)).toEqual(['manual', 'schedule', 'webhook', 'telegram', 'discord', 'slack']);
    expect(OUTPUT_TARGETS.map((k) => k.id)).toEqual(['notify', 'chat', 'telegram', 'discord', 'slack', 'file']);
  });
  it('links only messengers to connection state', () => {
    expect(isMessenger('slack')).toBe(true);
    expect(isMessenger('file')).toBe(false);
    expect(isLinked('telegram', { telegram: false })).toBe(false);
    expect(isLinked('telegram', { telegram: true })).toBe(true);
    expect(isLinked('webhook', {})).toBe(true);
  });
  it('defaults to manual and notify', () => {
    expect(triggerOf({})).toBe('manual');
    expect(targetOf({})).toBe('notify');
  });
});

describe('applyRemote', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('adds new nodes, flashes changed ones, fades removed ones and then drops them', () => {
    const model = new CanvasModel({ onchange: vi.fn() });
    const g = starterGraph();
    model.load(g);
    const [first, second, third] = g.nodes;
    const next = {
      nodes: [
        { ...first, fields: { ...first.fields, trigger: 'webhook' } },
        second,
        { id: 'nn', kind: 'prompt', x: 5, y: 5, fields: { text: 'новый' } },
      ],
      edges: [],
      strokes: [],
    };
    const diff = model.applyRemote(next);
    expect(diff.added).toEqual(['nn']);
    expect(diff.changed).toEqual([first.id]);
    expect(diff.removed).toEqual([third.id]);
    const ghost = model.nodes.find((n) => n.id === third.id);
    expect(ghost.data.leaving).toBe(true);
    // Уходящий узел не попадает в граф для сохранения.
    expect(model.getGraph().nodes.map((n) => n.id)).not.toContain(third.id);
    vi.advanceTimersByTime(400);
    expect(model.nodes.some((n) => n.id === third.id)).toBe(false);
    expect(model.nodes.find((n) => n.id === first.id).data.flash).toBe(true);
    vi.advanceTimersByTime(1500);
    expect(model.nodes.find((n) => n.id === first.id).data.flash).toBe(false);
  });

  it('does not make a history step or schedule a save', () => {
    const onchange = vi.fn();
    const model = new CanvasModel({ onchange });
    model.load(starterGraph());
    model.applyRemote({ nodes: [], edges: [], strokes: [] });
    expect(onchange).not.toHaveBeenCalled();
    expect(model.canUndo).toBe(false);
  });
});
