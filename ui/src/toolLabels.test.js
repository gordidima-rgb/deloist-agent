import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

const source = readFileSync(resolve(process.cwd(), '../web/tool-labels.js'), 'utf8');
let S;

beforeAll(() => {
  // eslint-disable-next-line no-new-func
  new Function(source).call(window);
  S = window.CrewToolLabels;
});

describe('plainName', () => {
  it('strips mcp prefixes in all three forms', () => {
    expect(S.plainName('mcp.server.tool_name')).toBe('tool_name');
    expect(S.plainName('mcp__server__tool_name')).toBe('tool_name');
    expect(S.plainName('mcp__server-name__tool_name')).toBe('tool_name');
  });
});

describe('label', () => {
  it('labels propose_team through an MCP prefix', () => {
    expect(S.label('mcp__deloist__propose_team')).toBe('Предлагает команду');
  });

  it('falls back to words for unknown names', () => {
    expect(S.label('some_tool_name')).toBe('Some tool name');
  });
});

describe('hidden', () => {
  it('hides ToolSearch in any form', () => {
    expect(S.hidden({ name: 'ToolSearch' })).toBe(true);
    expect(S.hidden({ name: 'mcp__x__tool_search' })).toBe(true);
  });
});

describe('target', () => {
  it('picks pattern', () => {
    expect(S.target({ args: { pattern: 'foo' } })).toBe('foo');
  });

  it('masks secret-looking values', () => {
    expect(S.target({ args: { command: 'sk-abcdef1234567890' } })).toBe('•••');
  });

  it('hides text typed into a password field', () => {
    expect(S.target({ name: 'browser_keyboard', args: { text: 'hunter2', label: 'Пароль' } })).toBe('••••••');
    expect(S.target({ name: 'mcp__deloist_browser_x__browser_keyboard', args: { text: 'привет', selector: '#msg' } })).toBe('«привет»');
  });

  it('shows only the host of an opened address', () => {
    expect(S.target({ name: 'browser_nav', args: { url: 'https://www.example.com/a?token=abc' } })).toBe('example.com');
  });

  it('masks a secret inside a command but keeps long paths', () => {
    const out = S.target({ name: 'run_command', args: { command: 'curl -H "Authorization: Bearer abcDEF123xyz" https://api.x' } });
    expect(out).not.toContain('abcDEF123xyz');
    expect(out).toContain('curl');
    expect(S.target({ args: { path: '/home/user/projects/deloist/agentsrc' } })).toBe('/home/user/projects/deloist/agentsrc');
    expect(S.target({ args: { command: 'echo aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3z' } })).toBe('echo •••');
  });

  it('names unknown tools in words and keeps human agent titles', () => {
    expect(S.label({ name: 'mcp__srv__some_new_tool' })).toBe('Some new tool');
    expect(S.label({ name: 'Read notes.txt' })).toBe('Read notes.txt');
  });

  it('skips apiKey', () => {
    expect(S.target({ args: { apiKey: 'secret', pattern: 'pat' } })).toBe('pat');
  });
});

describe('summary', () => {
  const plural = (n, one, few, many) => (n === 1 ? one : n >= 2 && n <= 4 ? few : many);
  const calls = (...names) => names.map((name) => ({ name }));

  it('counts steps by kind and says it in words', () => {
    expect(S.summary(calls('read_file', 'read_file', 'web_search', 'web_search', 'web_search'), plural)).toBe('прочитал 2 файла, искал 3 раза');
  });

  it('treats browser steps and agent kinds like their web and file equivalents', () => {
    const list = [{ name: 'mcp__b__browser_nav' }, { name: 'x', kind: 'edit' }, { name: 'y', kind: 'execute' }, { name: 'z', kind: 'think' }];
    expect(S.summary(list, plural)).toBe('открыл 1 страницу, записал 1 файл, запустил 1 команду');
  });

  it('shows the three biggest kinds and folds the rest into "ещё N"', () => {
    const list = calls('read_file', 'read_file', 'read_file', 'web_search', 'web_search', 'web_fetch', 'write_file', 'run_command', 'add_todo');
    expect(S.summary(list, plural)).toBe('прочитал 3 файла, искал 2 раза, открыл 1 страницу, ещё 3 шага');
  });

  it('skips hidden tools and is empty without steps', () => {
    expect(S.summary(calls('tool_search'), plural)).toBe('');
    expect(S.summary([], plural)).toBe('');
  });
});

describe('Claude Code helpers', () => {
  const plural = (n, one, few, many) => (n % 10 === 1 && n % 100 !== 11 ? one : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20) ? few : many);
  const helper = (id, extra = {}) => ({ id, name: 'Первая выборка', kind: 'subagent', tool: 'Agent', ...extra });

  it('labels the helper call and SendMessage by the adapter tool name, not the title', () => {
    expect(S.label(helper('acp-a'))).toBe('Зовёт помощника');
    expect(S.label({ name: 'SendMessage', kind: 'other', tool: 'SendMessage', args: { to: 'a1b2' } })).toBe('Пишет помощнику');
    expect(S.target({ name: 'SendMessage', tool: 'SendMessage', args: { to: 'a1b2' } })).toBe('a1b2');
    expect(S.label({ name: 'Read x.txt', kind: 'read', tool: 'Read' })).toBe('Читает файл');
  });

  it('puts child steps under their helper and leaves orphans on top', () => {
    const calls = [
      { id: 'acp-r', name: 'Read a', kind: 'read' },
      helper('acp-a'),
      { id: 'acp-c1', name: 'Grep', kind: 'search', parent: 'acp-a' },
      { id: 'acp-c2', name: 'Read b', kind: 'read', parent: 'acp-a' },
      { id: 'acp-o', name: 'Lost', kind: 'read', parent: 'acp-gone' },
    ];
    const tree = S.nest(calls);
    expect(tree.map((r) => r.call.id)).toEqual(['acp-r', 'acp-a', 'acp-o']);
    expect(tree[1].steps.map((r) => r.call.id)).toEqual(['acp-c1', 'acp-c2']);
  });

  it('counts helpers in the turn summary, not their inner steps', () => {
    const top = S.nest([helper('acp-a'), { id: 'acp-c', name: 'Read b', kind: 'read', parent: 'acp-a' }, helper('acp-b')]).map((r) => r.call);
    expect(S.summary(top, plural)).toBe('позвал помощника 2 раза');
  });
});
