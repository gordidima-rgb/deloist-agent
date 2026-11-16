import { describe, expect, it } from 'vitest';
import { buildGroups, filterGroups, toValue, isCurrent, labelOf, iconFor, iconSvg, subtitleOf } from './modelPicker.js';

const plan = { driver: 'deloist', defaultModel: 'deepseek-flash', models: [{ id: 'deepseek-flash', name: 'DeepSeek Flash' }, { id: 'gpt-6.1-terra', name: 'GPT-6.1 Terra' }] };
const providers = [
  { id: 'codex', kind: 'acp', name: 'ChatGPT', icon: 'x.svg', installed: '1.0', status: 'connected', enabled: true, driver: 'acp:codex', models: [{ id: 'gpt-5', name: 'GPT-5' }, { id: 'gpt-5-mini', name: 'GPT-5 mini' }], hiddenModels: ['gpt-5-mini'] },
  { id: 'openrouter', kind: 'key', name: 'OpenRouter', status: 'connected', enabled: true, driver: 'openai:openrouter', models: [{ id: 'a/b:free', name: 'Free One' }, { id: 'c/d', name: 'Paid Two' }], hiddenModels: [] },
  { id: 'off', kind: 'key', name: 'Выключен', status: 'connected', enabled: false, driver: 'openai:off', models: [{ id: 'z', name: 'Z' }] },
  { id: 'nokey', kind: 'key', name: 'БезКлюча', status: 'not_connected', enabled: true, driver: 'openai:nokey', models: [{ id: 'y', name: 'Y' }] },
];
const build = (value, settings = {}) => buildGroups({ providers, plan, settings, value });
const names = (groups) => groups.map((g) => g.name);

describe('grouping', () => {
  it('groups models by provider, subscription first, only connected and enabled', () => {
    const groups = build({ driver: '', model: '' });
    expect(names(groups).slice(0, 3)).toEqual(['Подписка Deloist', 'ChatGPT', 'OpenRouter']);
    expect(names(groups)).not.toContain('Выключен');
    expect(names(groups)).not.toContain('БезКлюча');
    expect(groups.find((g) => g.id === 'deloist').models.map((m) => m.badge)).toEqual(['sub', 'sub']);
  });

  it('marks free models and does not invent badges for others', () => {
    const or = build({ driver: '', model: '' }).find((g) => g.id === 'openrouter');
    expect(or.models.map((m) => [m.id, m.badge])).toEqual([['a/b:free', 'free'], ['c/d', '']]);
  });

  it('shows manual drivers only when they are set up or currently chosen', () => {
    expect(names(build({ driver: '', model: '' }))).not.toContain('Anthropic (Claude)');
    expect(names(build({ driver: '', model: '' }, { hasAnthropicKey: true }))).toContain('Anthropic (Claude)');
    expect(names(build({ driver: 'gemini', model: 'gemini-2.5-pro' }))).toContain('Google Gemini');
  });
});

describe('hidden models', () => {
  it('drops hiddenModels from the list', () => {
    const codex = build({ driver: '', model: '' }).find((g) => g.id === 'codex');
    expect(codex.models.map((m) => m.id)).toEqual(['gpt-5']);
  });

  it('still shows a hidden model when a saved bot uses it', () => {
    const groups = build({ driver: 'acp:codex', model: 'gpt-5-mini' });
    const codex = groups.find((g) => g.id === 'codex');
    expect(codex.models.map((m) => m.id)).toContain('gpt-5-mini');
    expect(labelOf(groups, { driver: 'acp:codex', model: 'gpt-5-mini' }).text).toBe('ChatGPT/gpt-5-mini');
  });
});

describe('search', () => {
  const groups = build({ driver: '', model: '' });
  it('matches model name and id, case-insensitive, and drops empty groups', () => {
    expect(names(filterGroups(groups, 'TERRA'))).toEqual(['Подписка Deloist']);
    expect(filterGroups(groups, 'c/d')[0].models.map((m) => m.id)).toEqual(['c/d']);
  });
  it('matches the provider name and keeps all its models', () => {
    const hit = filterGroups(groups, 'openrouter');
    expect(hit.map((g) => g.id)).toEqual(['openrouter']);
    expect(hit[0].models).toHaveLength(2);
  });
  it('filters to one provider from the rail', () => {
    expect(filterGroups(groups, '', 'codex').map((g) => g.id)).toEqual(['codex']);
  });
  it('turns typed text into a custom-model row for manual drivers', () => {
    const manual = build({ driver: '', model: '' }, { hasKey: true });
    const hit = filterGroups(manual, 'my-model-7').find((g) => g.driver === 'api');
    expect(hit.models).toEqual([{ id: 'my-model-7', name: 'my-model-7', badge: '', typed: true }]);
  });
});

describe('choosing a model', () => {
  it('sets driver and model exactly like the old two fields', () => {
    const groups = build({ driver: '', model: '' });
    const codex = groups.find((g) => g.id === 'codex');
    expect(toValue(codex, codex.models[0])).toEqual({ driver: 'acp:codex', model: 'gpt-5' });
    const sub = groups.find((g) => g.id === 'deloist');
    expect(toValue(sub, sub.models[1])).toEqual({ driver: 'deloist', model: 'gpt-6.1-terra' });
  });
});

describe('saved bots keep their model', () => {
  it('empty driver reads as «как в настройках»', () => {
    expect(labelOf(build({ driver: '', model: '' }), { driver: '', model: '' }).text).toBe('как в настройках');
  });
  it('subscription with an empty model shows the default model and checks it', () => {
    const value = { driver: 'deloist', model: '' };
    const groups = build(value);
    expect(labelOf(groups, value).text).toBe('Подписка Deloist/DeepSeek Flash');
    const sub = groups.find((g) => g.id === 'deloist');
    expect(sub.models.filter((m) => isCurrent(sub, m, value)).map((m) => m.id)).toEqual(['deepseek-flash']);
  });
  it('a hand-typed model on a legacy driver stays visible and checked', () => {
    const value = { driver: 'anthropic', model: 'claude-x' };
    const groups = build(value);
    expect(labelOf(groups, value).text).toBe('Anthropic (Claude)/claude-x');
    const g = groups.find((x) => x.driver === 'anthropic');
    expect(g.models.some((m) => isCurrent(g, m, value))).toBe(true);
  });
  it('a removed acp agent still shows what it was', () => {
    const value = { driver: 'acp:gone', model: 'm1' };
    expect(labelOf(build(value), value).text).toBe('gone/m1');
  });
  it('demo driver ignores a leftover model name', () => {
    for (const model of ['', 'gpt-4o-mini']) {
      const value = { driver: 'demo', model };
      const groups = build(value);
      expect(labelOf(groups, value).text).toBe('Демо, без ключа');
      const demo = groups.find((g) => g.driver === 'demo');
      expect(demo.models.filter((m) => isCurrent(demo, m, value))).toHaveLength(1);
    }
  });
});

describe('only connected providers are listed', () => {
  const mk = (id, extra) => ({ id, kind: 'key', name: id, status: 'connected', enabled: true, driver: `openai:${id}`, models: [{ id: 'x', name: 'X' }], ...extra });
  it('shows connected, hides not loaded and not connected', () => {
    const groups = buildGroups({
      providers: [mk('ok'), mk('no', { status: 'not_connected' }), { id: 'ag', kind: 'acp', name: 'Agent', installed: '', status: 'not_installed', enabled: true, driver: 'acp:ag', models: [] }, mk('local', { local: true })],
      settings: {},
      value: { driver: '', model: '' },
    });
    expect(groups.map((g) => g.id)).toEqual(['ok', 'local']);
  });
  it('keeps the current broken value, marked as not connected', () => {
    const groups = buildGroups({ providers: [mk('ok'), mk('no', { status: 'not_connected' })], settings: {}, value: { driver: 'openai:no', model: 'x' } });
    const broken = groups.find((g) => g.driver === 'openai:no');
    expect(broken.kind).toBe('missing');
    expect(subtitleOf(broken)).toBe('Не подключено');
  });
  it('shows demo only when nothing else is connected (or it is current)', () => {
    expect(buildGroups({ providers: [mk('ok')], settings: {}, value: { driver: '', model: '' } }).some((g) => g.driver === 'demo')).toBe(false);
    expect(buildGroups({ providers: [], settings: {}, value: { driver: '', model: '' } }).some((g) => g.driver === 'demo')).toBe(true);
    expect(buildGroups({ providers: [mk('ok')], settings: {}, value: { driver: 'demo', model: '' } }).some((g) => g.driver === 'demo')).toBe(true);
  });
});

describe('logos', () => {
  it('maps groups to icon keys', () => {
    expect(iconFor({ id: 'deloist', driver: 'deloist' })).toBe('deloist');
    expect(iconFor({ id: 'anthropic', driver: 'anthropic' })).toBe('claude');
    expect(iconFor({ id: 'ollama-cloud', driver: 'openai:ollama-cloud' })).toBe('ollama');
    expect(iconFor({ id: 'claude-agent-acp', driver: 'acp:claude-agent-acp', kind: 'acp' })).toBe('claude');
    expect(iconFor({ id: 'codex', driver: 'acp:codex' })).toBe('openai');
    expect(iconFor({ id: 'alice', driver: 'openai:alice' })).toBe('yandex');
    expect(iconFor({ id: 'weird', driver: 'openai:weird' })).toBeNull();
    expect(iconSvg({ id: 'cli', driver: 'cli' })).toContain('<svg');
  });
});

describe('agent aliases and ACP status', () => {
  const acp = (extra) => ({ id: 'claude-agent-acp', kind: 'acp', name: 'Claude Code', installed: '1', status: 'connected', enabled: true, driver: 'acp:claude-agent-acp', models: [{ id: 'sonnet', name: 'Sonnet' }, { id: 'default', name: 'Default (recommended)' }], ...extra });
  it('shows the «default» alias first as a readable row with a chip', () => {
    const g = buildGroups({ providers: [acp()], settings: {}, value: { driver: '', model: '' } }).find((x) => x.id === 'claude-agent-acp');
    expect(g.models[0]).toMatchObject({ id: 'default', name: 'По умолчанию — выбирает Claude Code', badge: 'default' });
  });
  it('hides an ACP agent that is installed but not connected', () => {
    const ids = buildGroups({ providers: [acp({ status: 'not_connected' })], settings: {}, value: { driver: '', model: '' } }).map((x) => x.id);
    expect(ids).not.toContain('claude-agent-acp');
  });
});
