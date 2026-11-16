// Лента хода идёт по времени: шаги -> карточка -> шаги даёт группу, карточку, группу.
// Код живёт в web/app.js (старый DOM-экран), тест вырезает блок и запускает его с заглушками.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const source = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');
const block = source.slice(source.indexOf('let workGroup = null;'), source.indexOf('function clearMessages()'));

function feed() {
  const root = document.createElement('div');
  document.body.replaceChildren(root);
  const api = new Function(
    'messagesEl', 'ICONS', 'tr', '$$', 'turnCard',
    `${block}\nreturn { ensureWorkGroup, paintWorkNow, appendInFeed, finishWorkGroup, group: () => workGroup };`,
  )(
    () => root,
    { chev: '<i></i>' },
    (s) => s,
    (sel, r = document) => [...r.querySelectorAll(sel)],
    { status() {} },
  );
  const step = (text) => {
    const row = document.createElement('div');
    row.className = 'msg bot';
    row.textContent = text;
    api.ensureWorkGroup().querySelector('.work-body').append(row);
    api.paintWorkNow('act', text);
  };
  const card = (name) => {
    const c = document.createElement('div');
    c.className = 'card ' + name;
    api.appendInFeed(c);
  };
  return { root, api, step, card };
}

const shape = (root) => [...root.children].map((e) => (e.classList.contains('work-group') ? 'group' + (e.classList.contains('is-live') ? '*' : '') : e.className.split(' ')[1]));

describe('chronological feed', () => {
  it('steps, card, steps gives group, card, group, and only the last group is live', () => {
    const { root, step, card } = feed();
    step('Читает файл');
    card('approval');
    step('Пишет файл');
    expect(shape(root)).toEqual(['group', 'approval', 'group*']);
    expect(root.children[0].querySelector('.work-body').textContent).toBe('Читает файл');
    expect(root.children[2].querySelector('.work-now').textContent).toBe('Пишет файл');
  });

  it('puts the live indicator below a card that arrives while waiting', () => {
    const { root, step, card } = feed();
    step('Открывает страницу');
    card('approval');
    expect(shape(root)).toEqual(['group', 'approval', 'group*']);
    expect(root.lastElementChild.querySelector('.work-now').textContent).toBe('Думает');
  });

  it('keeps cards outside a running turn where they are', () => {
    const { root, card, api } = feed();
    card('question');
    expect(shape(root)).toEqual(['question']);
    api.ensureWorkGroup();
    card('approval');
    expect(shape(root)).toEqual(['question', 'group', 'approval']);
  });

  it('calms the live group when the turn ends', () => {
    const { root, step, api } = feed();
    step('Читает файл');
    api.finishWorkGroup();
    expect(shape(root)).toEqual(['group']);
  });
});

// Свёрнутый «Ход работы» прячет технику, но не слова бота человеку (bubble). Само скрытие делает CSS,
// поэтому проверяем и разметку, и правила: текст ответа лежит в .bubble внутри .work-body, шаги в .tools.
describe('collapsed work log keeps the bot prose visible', () => {
  const css = readFileSync(resolve(process.cwd(), '../web/style.css'), 'utf8');
  const rule = (selector) => css.split(String.fromCharCode(10)).filter((l) => l.includes(selector));

  it('shows the closed details content and hides only tools, meta and rows without a bubble', () => {
    expect(rule('.work-group:not([open])::details-content').join(' ')).toContain('content-visibility: visible');
    const hidden = rule('.work-group:not([open]) .work-body').join(' ');
    expect(hidden).toContain('.tools');
    expect(hidden).toContain('.msg-meta');
    expect(hidden).toContain('.msg:not(:has(.bubble:not(:empty)))');
    expect(hidden).not.toMatch(/\.bubble\s*[,{][^}]*display:\s*none/);
  });

  it('keeps prose and steps of a live row in separate elements', () => {
    const { api } = feed();
    const row = document.createElement('div');
    row.className = 'msg bot';
    row.innerHTML = '<div class="msg-col"><div class="tools live"><details class="tool"></details></div><div class="bubble">Добавлю двух ботов</div></div>';
    api.ensureWorkGroup().querySelector('.work-body').append(row);
    const body = api.group().querySelector('.work-body');
    expect(body.querySelector('.bubble').closest('.tools')).toBeNull();
    expect(body.querySelector('.tools')).not.toBeNull();
  });
});
