// Лента хода идёт по времени: шаги -> карточка -> шаги даёт группу, карточку, группу.
// Код живёт в web/app.js (старый DOM-экран), тест вырезает блок и запускает его с заглушками.
import { beforeAll, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const labels = readFileSync(resolve(process.cwd(), '../web/tool-labels.js'), 'utf8');
const plural = (n, one, few, many) => (n === 1 ? one : n < 5 ? few : many);

beforeAll(() => {
  new Function(labels).call(window);
});

const source = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');
const block = source.slice(source.indexOf('let workGroup = null;'), source.indexOf('function clearMessages()'));
const liveBlock = source.slice(source.indexOf('function splitLive('), source.indexOf('// Строка хода агента'));

function feed() {
  const root = document.createElement('div');
  document.body.replaceChildren(root);
  const S = { live: null };
  const api = new Function(
    'messagesEl', 'ICONS', 'tr', '$$', 'turnCard', 'S', 'md', 'scrollBottom', 'startLive', 'plural',
    `${block}\n${liveBlock}\nreturn { ensureWorkGroup, paintWorkNow, appendInFeed, finishWorkGroup, appendDelta, paintLiveSteps, sealWork, group: () => workGroup };`,
  )(
    () => root,
    { chev: '<i></i>' },
    (s, vars) => String(s).replace(/\{(\w+)\}/g, (m, k) => (vars && k in vars ? vars[k] : m)),
    (sel, r = document) => [...r.querySelectorAll(sel)],
    { status() {} },
    S,
    (s) => s,
    () => {},
    function startLive(turnId, step) {
      if (S.live) return S.live;
      const row = document.createElement('div');
      row.className = 'msg bot';
      const col = document.createElement('div');
      const bubble = document.createElement('div');
      bubble.className = 'bubble cursor';
      col.append(bubble);
      row.append(col);
      api.ensureWorkGroup().querySelector('.work-body').append(row);
      S.live = { turnId, step, row, col, bubble, text: '' };
      return S.live;
    },
    plural,
  );
  const step = (text) => {
    const row = document.createElement('div');
    row.className = 'msg bot';
    row.textContent = text;
    api.ensureWorkGroup().querySelector('.work-body').append(row);
    api.paintWorkNow('act', text);
  };
  const tool = (name, status = 'ok', args = {}) => {
    const node = document.createElement('details');
    node.className = 'tool';
    node.dataset.status = status;
    node._call = { id: name + Math.random(), name, args, status };
    node.innerHTML = '<summary><i class="t-chev"></i><span class="t-label">' + name + '</span><span class="t-status">' + status + '</span></summary>';
    const row = document.createElement('div');
    row.className = 'msg bot';
    row.append(node);
    api.ensureWorkGroup().querySelector('.work-body').append(row);
    api.paintWorkNow('act', name);
    return node;
  };
  const card = (name) => {
    const c = document.createElement('div');
    c.className = 'card ' + name;
    api.appendInFeed(c);
  };
  return { root, api, step, card, tool };
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

  it('text streamed after a card goes below it, not into the bubble above', () => {
    const { root, card, api } = feed();
    api.appendDelta({ turnId: 't', step: 0, text: 'Сначала сверю файл.' });
    card('question');
    api.appendDelta({ turnId: 't', step: 0, text: ' Человек подтвердил.' });
    expect(shape(root)).toEqual(['group', 'question', 'group*']);
    expect(root.children[0].querySelector('.bubble').textContent).toBe('Сначала сверю файл.');
    expect(root.children[0].querySelector('.bubble').classList.contains('cursor')).toBe(false);
    expect(root.children[2].querySelector('.bubble').textContent).toBe(' Человек подтвердил.');
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
    expect(hidden).toContain('.tools:not(:has(');
    expect(hidden).toContain('.msg-meta');
    expect(hidden).toContain('.msg:not(:has(.bubble:not(:empty)');
    // упавший шаг остаётся на виду и в свёрнутом ходе
    expect(hidden).toContain('data-status="error"');
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

describe('turn summary row', () => {
  it('names what the turn did in one row when a group ends, and merges the turn stats into it', () => {
    const { root, tool, api } = feed();
    tool('read_file');
    tool('read_file');
    tool('web_search');
    tool('web_search');
    tool('web_search');
    api.finishWorkGroup();
    const label = () => root.querySelector('.work-group > summary span').textContent;
    expect(label()).toBe('Работал · прочитал 2 файла, искал 3 раза');
    api.sealWork(root.querySelector('.work-group'), { ms: 29000, meta: '1 200 токенов' });
    expect(label()).toBe('Работал {t} · прочитал 2 файла, искал 3 раза'.replace('{t}', '{n} с'.replace('{n}', (29).toLocaleString(window.LOCALE))));
    expect(root.querySelector('.work-meta').textContent).toBe('1 200 токенов');
  });

  it('marks failed steps in the row', () => {
    const { root, tool, api } = feed();
    tool('run_command');
    tool('run_command', 'error');
    api.finishWorkGroup();
    const group = root.querySelector('.work-group');
    expect(group.classList.contains('has-failed')).toBe(true);
    expect(group.querySelector('summary span').textContent).toContain('с ошибкой: 1');
  });

  it('shows the last six steps while running and counts the rest', () => {
    const { root, tool, api } = feed();
    for (let i = 0; i < 9; i += 1) tool('read_file');
    api.paintLiveSteps(api.group());
    const live = root.querySelector('.work-live');
    expect(live.querySelectorAll('.work-step').length).toBe(6);
    expect(live.querySelector('.work-more').textContent).toBe('ещё 3 шага');
    api.finishWorkGroup();
    expect(root.querySelector('.work-live')).toBeNull();
  });
});
