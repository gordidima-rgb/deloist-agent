// Ход работы: схлопывание шагов и подписи целей. Код живёт в web/app.js (старый DOM-экран),
// поэтому тест вырезает блок между маркерами и запускает его с заглушками.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const source = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');
const from = source.indexOf('// Агенты ACP зовут наш браузер через MCP');
const to = source.indexOf('// Ошибка говорит по-человечески');
const block = source.slice(from, to);

function load() {
  const factory = new Function(
    'tr', 'escapeHtml', 'statusLabel', 'plural', 'ICONS', 'TOOL_LABELS',
    `${block}\nreturn { toolTarget, toolNode, addTool, refreshGroup, toolFamily, toolLabel, liveStepText };`,
  );
  return factory(
    (s) => s,
    (s) => String(s),
    (s) => s,
    (n, one, few, many) => (n === 1 ? one : n < 5 ? few : many),
    { chev: '<i class="chev"></i>' },
    { browser_nav: 'Открывает страницу', browser_page: 'Читает страницу', browser_mouse: 'Двигает мышью', browser_keyboard: 'Печатает', web_search: 'Ищет в интернете', read_file: 'Читает файл' },
  );
}

const call = (name, args, status = 'ok', id = name + Math.random()) => ({ id, name, args, status });

describe('chat tool steps', () => {
  it('names a meaningful target from the call args', () => {
    const { toolTarget } = load();
    expect(toolTarget(call('browser_nav', { url: 'https://www.example.com/a?b=1' }))).toBe('example.com');
    expect(toolTarget(call('mcp.deloist_browser.browser_nav', { url: 'https://habr.com/ru' }))).toBe('habr.com');
    expect(toolTarget(call('web_search', { query: 'цены на кружки' }))).toBe('цены на кружки');
    expect(toolTarget(call('browser_mouse', { action: 'click', label: 'Экспорт' }))).toBe('Экспорт');
    expect(toolTarget(call('browser_keyboard', { action: 'type', text: 'привет' }))).toBe('«привет»');
    expect(toolTarget(call('browser_keyboard', { action: 'type', text: 's3cret', label: 'Пароль' }))).toBe('••••••');
    expect(toolTarget(call('browser_page', {}))).toBe('');
  });

  it('never shows an empty {} block', () => {
    const { toolNode } = load();
    const empty = toolNode(call('browser_page', {}));
    expect(empty.querySelector('.tool-body').children.length).toBe(0);
    const full = toolNode(call('read_file', { path: 'a.txt' }));
    expect(full.querySelectorAll('.tool-body pre').length).toBe(1);
  });

  it('folds consecutive browser steps into one expandable row', () => {
    const { toolNode, addTool } = load();
    const tools = document.createElement('div');
    addTool(tools, toolNode(call('browser_nav', { url: 'https://example.com' }, 'running')));
    addTool(tools, toolNode(call('mcp.deloist_browser.browser_page', {})));
    addTool(tools, toolNode(call('browser_mouse', { label: 'Найти' })));
    expect(tools.children.length).toBe(1);
    const group = tools.firstElementChild;
    expect(group.classList.contains('tool-grp')).toBe(true);
    expect(group.querySelectorAll('.tool').length).toBe(3);
    expect(group.querySelector('summary .t-label').textContent).toBe('Работает в браузере');
    expect(group.querySelector('summary .t-target').textContent).toBe('3 шага · example.com');
    expect(group.dataset.status).toBe('running');
  });

  it('keeps a different tool between browser steps as its own row', () => {
    const { toolNode, addTool } = load();
    const tools = document.createElement('div');
    addTool(tools, toolNode(call('browser_nav', { url: 'https://a.io' })));
    addTool(tools, toolNode(call('read_file', { path: 'x.txt' })));
    addTool(tools, toolNode(call('browser_nav', { url: 'https://b.io' })));
    expect(tools.children.length).toBe(3);
  });

  it('folds repeated identical tools and reports an error status of any step', () => {
    const { toolNode, addTool, refreshGroup } = load();
    const tools = document.createElement('div');
    addTool(tools, toolNode(call('read_file', { path: 'a.txt' })));
    addTool(tools, toolNode(call('read_file', { path: 'b.txt' }, 'error')));
    const group = tools.firstElementChild;
    refreshGroup(group);
    expect(group.dataset.status).toBe('error');
    expect(group.querySelector('summary .t-label').textContent).toBe('Читает файл');
  });

  it('turns an agent title that is a whole script into a short human label (never code or JSON)', () => {
    const { toolLabel, liveStepText, toolTarget, toolNode } = load();
    const script = 'Execute ' + String.fromCharCode(96) + "$ErrorActionPreference = 'Stop'" + String.fromCharCode(10) + "$root = 'C:" + String.fromCharCode(92) + "Users'" + String.fromCharCode(10) + 'x'.repeat(500) + String.fromCharCode(96);
    const run = { id: 'a', name: script, kind: 'execute', status: 'running', args: { command: script.slice(8) } };
    expect(toolLabel(run)).toBe('Запускает команду');
    expect(liveStepText(run, toolLabel(run), toolTarget(run))).toBe('Запускает команду');
    const json = { id: 'b', name: '{"botIds":["b1"],"lead":"x"}', kind: 'other', status: 'running', args: {} };
    expect(toolLabel(json)).toBe('Выполняет шаг');
    const quoted = { id: 'c', name: '"botIds":["b1"]', status: 'running' };
    expect(toolLabel(quoted)).not.toContain('botIds');
    const path = 'C:/w/x-lenta.md';
    const write = { id: 'd', name: 'Write ' + String.fromCharCode(96) + path + String.fromCharCode(96) + ' (unreadable, no preview)', kind: 'edit', status: 'ok', args: { path } };
    expect(toolLabel(write)).toBe('Записывает файл');
    expect(liveStepText(write, toolLabel(write), toolTarget(write))).toBe('Записывает файл x-lenta.md');
    // Подпись в строке шага и подсказка короткие, даже если заголовок огромный.
    const node = toolNode(run);
    expect(node.querySelector('.t-label').textContent).toBe('Запускает команду');
    expect(node.querySelector('.t-label').title.length).toBeLessThanOrEqual(160);
    // Короткий понятный заголовок агента остаётся как есть.
    expect(toolLabel({ id: 'e', name: 'Search tools: "ask_user"', status: 'ok' })).toBe('Search tools: "ask_user"');
  });
});
