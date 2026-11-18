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
