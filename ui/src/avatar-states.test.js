import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

// avatar-states.js — обычный скрипт страницы: кладёт всё в window.CrewAvatarStates.
const source = readFileSync(resolve(process.cwd(), '../web/avatar-states.js'), 'utf8');
const index = JSON.parse(readFileSync(resolve(process.cwd(), '../web/icons/apps/index.json'), 'utf8'));
let S;

beforeAll(() => {
  // eslint-disable-next-line no-new-func
  new Function(source).call(window);
  S = window.CrewAvatarStates;
  S.setApps(index);
});

describe('toolLook', () => {
  it('рисует: генерация картинок', () => {
    expect(S.toolLook({ name: 'generate_image', args: { prompt: 'кот' } })).toEqual({ act: 'draw' });
    expect(S.toolLook({ name: 'image', args: {} })).toEqual({ act: 'draw' });
  });

  it('рисует: навык imagegen и работа с картинками у внешнего агента', () => {
    expect(S.toolLook({ name: 'execute', kind: 'execute', args: { command: 'codex exec --skill imagegen "кадр"' } })).toEqual({ act: 'draw' });
    expect(S.toolLook({ name: 'read', kind: 'read', title: 'Читает файл Лавовый шар.png', args: {} })).toEqual({ act: 'draw' });
    expect(S.toolLook({ name: 'read_file', args: { path: 'attachments/снимок.jpg' } })).toEqual({ act: 'draw' });
    expect(S.toolLook({ name: 'read_file', args: { path: 'notes.md' } })).toEqual({ act: 'doc' });
  });

  it('ищет: поиск в сети и браузер, а у внешних агентов вид search и fetch', () => {
    expect(S.toolLook({ name: 'web_search', args: { query: 'x' } })).toEqual({ act: 'search' });
    expect(S.toolLook({ name: 'browser_nav', args: { url: 'https://github.com' } })).toEqual({ act: 'app', icon: 'icons/apps/github.svg' });
    expect(S.toolLook({ name: 'Поиск', kind: 'search' })).toEqual({ act: 'search' });
    expect(S.toolLook({ name: 'Fetch', kind: 'fetch' })).toEqual({ act: 'search' });
  });

  it('ищет сервис: запрос называет сервис — на шаре его значок', () => {
    const app = (slug) => ({ act: 'app', icon: `icons/apps/${slug}.svg` });
    expect(S.toolLook({ name: 'web_search', args: { query: 'OpenBot nightly-labs github' } })).toEqual(app('github'));
    expect(S.toolLook({ name: 'google_web_search', kind: 'search', args: { query: 'youtube channel stats' } })).toEqual(app('youtube'));
    expect(S.toolLook({ name: 'Searching the web for: notion api limits', kind: 'search' })).toEqual(app('notion'));
    expect(S.toolLook({ name: 'web_search', args: { query: 'twitter api pricing' } })).toEqual(app('x'));
    expect(S.toolLook({ name: 'web_search', args: { query: 'как варить борщ' } })).toEqual({ act: 'search' });
    expect(S.toolLook({ name: 'web_search', args: { query: 'python tutorial' } })).toEqual({ act: 'search' });
    expect(S.toolLook({ name: 'browser_page', args: { query: 'github' } })).toEqual({ act: 'search' });
  });

  it('собирает: команды сборки и компиляции', () => {
    for (const command of ['cargo build --release', 'npm run build', 'cmake --build .', 'dotnet build', 'tsc -p .', 'make all', 'gradlew assemble']) {
      expect(S.toolLook({ name: 'run_command', args: { command } }), command).toEqual({ act: 'build' });
    }
    expect(S.toolLook({ name: 'run_command', args: { command: 'git status' } })).toBeNull();
    expect(S.toolLook({ name: 'run_command', args: { command: 'ls -la' } })).toBeNull();
  });

  it('пишет документ, таблицу, PDF: по расширению файла', () => {
    expect(S.toolLook({ name: 'write_file', args: { path: 'report.docx' } })).toEqual({ act: 'doc' });
    expect(S.toolLook({ name: 'write_file', args: { path: 'notes/readme.md' } })).toEqual({ act: 'doc' });
    expect(S.toolLook({ name: 'read_file', args: { path: 'a.txt' } })).toEqual({ act: 'doc' });
    expect(S.toolLook({ name: 'write_file', args: { path: 'budget.xlsx' } })).toEqual({ act: 'sheet' });
    expect(S.toolLook({ name: 'write_file', args: { path: 'rows.csv' } })).toEqual({ act: 'sheet' });
    expect(S.toolLook({ name: 'write_file', args: { path: 'invoice.PDF' } })).toEqual({ act: 'pdf' });
    expect(S.toolLook({ name: 'run_command', args: { command: 'libreoffice --convert-to pdf "out.pdf"' } })).toEqual({ act: 'pdf' });
    expect(S.toolLook({ name: 'write_file', args: { path: 'main.rs' } })).toBeNull();
  });

  it('работает в приложении: мессенджеры и программы из списка значков', () => {
    const app = (slug) => ({ act: 'app', icon: `icons/apps/${slug}.svg` });
    expect(S.toolLook({ name: 'telegram_posts', args: {} })).toEqual(app('telegram'));
    expect(S.toolLook({ name: 'mcp__discord__send', args: {} })).toEqual(app('discord'));
    expect(S.toolLook({ name: 'mcp__slack__post', args: {} })).toEqual(app('slack'));
    expect(S.toolLook({ name: 'run_command', args: { command: 'godot --headless --export-release' } })).toEqual(app('godotengine'));
    expect(S.toolLook({ name: 'Unity', kind: 'execute', args: {} })).toEqual(app('unity'));
    expect(S.toolLook({ name: 'run_command', args: { command: 'blender -b scene.blend -a' } })).toEqual(app('blender'));
    expect(S.toolLook({ name: 'mcp__figma__get_file', args: {} })).toEqual(app('figma'));
  });

  it('каждый значок, на который указывает знак, лежит в списке', () => {
    for (const slug of Object.keys(index)) {
      const look = S.toolLook({ name: `mcp__${slug}__x`, args: {} });
      if (look) expect(look.icon).toBe(`icons/apps/${slug}.svg`);
    }
  });

  it('общие слова не считаются приложением', () => {
    for (const command of ['git commit -m x', 'python script.py', 'node app.js', 'docker ps']) {
      expect(S.toolLook({ name: 'run_command', args: { command } }), command).toBeNull();
    }
  });
});

describe('moodFor и waitOf', () => {
  it('эмоции по событиям, у которых есть сигнал', () => {
    expect(S.moodFor({ type: 'approval_closed', status: 'denied' })).toEqual({ mood: 'sad', seconds: 2 });
    expect(S.moodFor({ type: 'approval_closed', status: 'allowed' })).toBeNull();
    expect(S.moodFor({ type: 'model_retry' }).mood).toBe('grumpy');
    expect(S.moodFor({ type: 'model_switched' }).mood).toBe('surprise');
    expect(S.moodFor({ type: 'compacting' }).mood).toBe('dizzy');
    expect(S.moodFor({ type: 'memory_learned' }).mood).toBe('love');
    expect(S.moodFor({ type: 'skill_learned' }).mood).toBe('star');
    expect(S.moodFor({ type: 'text_delta' })).toBeNull();
  });

  it('ошибки хода: лимит, перегрузка, недоступно, остальное', () => {
    expect(S.errorMood('HTTP 429 Too Many Requests').mood).toBe('sleep');
    expect(S.errorMood('model overloaded').mood).toBe('dizzy');
    expect(S.errorMood('503 bad gateway').mood).toBe('dizzy');
    expect(S.errorMood('Эта модель сейчас недоступна').mood).toBe('sad');
    expect(S.errorMood('что-то странное').mood).toBe('error');
    expect(S.errorMood(undefined).mood).toBe('error');
  });

  it('ожидание: одобрение, вопрос, просьба посмотреть браузер, предложение специалиста', () => {
    expect(S.waitOf({ type: 'approval', id: 'a1' })).toEqual({ open: 'a1' });
    expect(S.waitOf({ type: 'question', id: 'q1' })).toEqual({ open: 'q1' });
    expect(S.waitOf({ type: 'browser_needs_human', botId: 'b' })).toEqual({ open: 'human' });
    expect(S.waitOf({ type: 'specialist_offer', offer: { id: 'o1' } })).toEqual({ open: 'offer:o1' });
    expect(S.waitOf({ type: 'approval_closed', id: 'a1' })).toEqual({ close: 'a1' });
    expect(S.waitOf({ type: 'specialist_offer' })).toBeNull();
    expect(S.waitOf({ type: 'team_proposal', proposal: { id: 'o2' } })).toEqual({ open: 'offer:o2' });
    expect(S.waitOf({ type: 'team_proposal' })).toBeNull();
    expect(S.waitOf({ type: 'turn_start' })).toBeNull();
  });
});

describe('createTracker', () => {
  const make = () => {
    const log = [];
    const avatar = { set: (o) => log.push(['set', o]), feel: (m, s) => log.push(['feel', m, s]) };
    let now = 0;
    const tracker = S.createTracker(() => [avatar], () => now);
    return { tracker, log, tick: (ms) => { now += ms; } };
  };
  const act = (log) => log.filter(([kind]) => kind === 'set').map(([, o]) => o.act);

  it('знак идёт за вызовами и возвращается к обычной загрузке', () => {
    const { tracker, log } = make();
    tracker.feed({ type: 'turn_start', botId: 'b' });
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'c1', name: 'generate_image', args: {} });
    tracker.feed({ type: 'tool_update', botId: 'b', callId: 'c1', status: 'ok' });
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'c2', name: 'write_file', args: { path: 'a.pdf' } });
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'c3', name: 'run_command', args: { command: 'ls' } });
    tracker.feed({ type: 'turn_end', botId: 'b' });
    expect(act(log)).toEqual(['', 'draw', '', 'pdf', '', '']);
  });

  it('значок приложения передаётся адресом', () => {
    const { tracker, log } = make();
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'c1', name: 'telegram_posts', args: {} });
    expect(log.find(([k]) => k === 'set')[1]).toEqual({ act: 'app', icon: 'icons/apps/telegram.svg' });
  });

  it('три ошибки подряд — головокружение, успех сбрасывает счёт', () => {
    const { tracker, log, tick } = make();
    const fail = (id) => { tracker.feed({ type: 'tool_start', botId: 'b', callId: id, name: 'x' }); tracker.feed({ type: 'tool_update', botId: 'b', callId: id, status: 'error' }); };
    fail('1'); fail('2');
    expect(log.filter(([k]) => k === 'feel').map((e) => e[1])).toEqual(['grumpy', 'grumpy']);
    fail('3');
    expect(log.filter(([k]) => k === 'feel').map((e) => e[1])).toEqual(['grumpy', 'grumpy', 'dizzy']);
    tracker.feed({ type: 'tool_start', botId: 'b', callId: '4', name: 'x' });
    tick(5000);
    tracker.feed({ type: 'tool_update', botId: 'b', callId: '4', status: 'ok' });
    fail('5'); fail('6');
    expect(log.filter(([k]) => k === 'feel').map((e) => e[1]).slice(-3)).toEqual(['joy', 'grumpy', 'grumpy']);
  });

  it('радость от успеха не чаще раза в две секунды', () => {
    const { tracker, log, tick } = make();
    const ok = (id) => { tracker.feed({ type: 'tool_start', botId: 'b', callId: id, name: 'x' }); tracker.feed({ type: 'tool_update', botId: 'b', callId: id, status: 'ok' }); };
    ok('1'); tick(500); ok('2'); tick(1000); ok('3'); tick(600); ok('4');
    expect(log.filter(([k]) => k === 'feel')).toEqual([['feel', 'joy', 0.5], ['feel', 'joy', 0.5]]);
  });

  it('отказ и остановка не считаются ошибкой инструмента', () => {
    const { tracker, log } = make();
    tracker.feed({ type: 'tool_start', botId: 'b', callId: '1', name: 'x' });
    tracker.feed({ type: 'tool_update', botId: 'b', callId: '1', status: 'denied' });
    expect(log.filter(([k]) => k === 'feel')).toEqual([]);
  });

  it('внешний агент: строка вызова обновляется, считаются только переходы', () => {
    const { tracker, log } = make();
    const row = (status, extra = {}) => ({ type: 'activity', botId: 'b', call: { id: 'acp-1', name: 'Search docs', kind: 'search', status, ...extra } });
    tracker.feed(row('running'));
    tracker.feed(row('running'));
    tracker.feed(row('ok'));
    tracker.feed(row('ok'));
    expect(act(log)).toEqual(['search', '']);
    expect(log.filter(([k]) => k === 'feel')).toEqual([['feel', 'joy', 0.5]]);
  });

  it('ACP и браузерный мост: X виден при работе и исчезает при завершении', () => {
    const { tracker, log } = make();
    const feed = (event) => tracker.feed({ botId: 'b', ...event });
    const last = () => log.filter(([k]) => k === 'set').at(-1)[1];
    feed({ type: 'turn_start' });
    feed({ type: 'activity', call: { id: 'acp-1', name: 'mcp.deloist_browser.browser_nav', kind: 'execute', status: 'running', args: {} } });
    expect(last().act).toBe('search');
    feed({ type: 'tool_start', callId: 'bridge', name: 'browser_nav', args: { action: 'open', url: 'https://x.com/example' } });
    expect(last()).toEqual({ act: 'app', icon: 'icons/apps/x.svg' });
    feed({ type: 'tool_update', callId: 'bridge', status: 'ok' });
    expect(last().act).toBe('app');
    feed({ type: 'activity', call: { id: 'acp-1', name: 'mcp.deloist_browser.browser_nav', kind: 'execute', status: 'ok', args: {} } });
    expect(last().act).toBe('');
    feed({ type: 'tool_start', callId: 'page', name: 'browser_page', args: { action: 'read' } });
    expect(last().icon).toBe('icons/apps/x.svg');
    feed({ type: 'tool_update', callId: 'page', status: 'ok' });
    expect(last().act).toBe('');
    feed({ type: 'tool_start', callId: 'other', name: 'browser_nav', args: { action: 'open', url: 'https://example.com' } });
    expect(last().act).toBe('search');
    feed({ type: 'turn_end' });
    expect(last().act).toBe('');
    expect(log.some(([k, mood]) => k === 'feel' && mood === 'joy')).toBe(true);
  });

  it('поздний URL в ACP обновляет значок и не принимает поддельный домен', () => {
    const { tracker, log } = make();
    const row = (args) => ({ type: 'activity', botId: 'b', call: { id: '1', name: 'Run mcp_deloist_browser_browser_nav', kind: 'other', status: 'running', args } });
    tracker.feed(row({}));
    tracker.feed(row({ url: 'https://mobile.twitter.com/a', action: 'open' }));
    expect(log.at(-1)[1].icon).toBe('icons/apps/x.svg');
    tracker.feed(row({ url: 'https://x.com.example.org', action: 'open' }));
    expect(log.at(-1)[1].act).toBe('search');
  });

  it('распознаёт сайты по hostname, не по логину, пути или параметрам', () => {
    for (const [url, slug] of [['https://github.com/a', 'github'], ['https://www.youtube.com/watch?v=1', 'youtube'], ['https://web.telegram.org/k/', 'telegram'], ['https://drive.google.com/', 'googledrive']]) {
      expect(S.toolLook({ name: 'browser_nav', args: { url } })).toEqual({ act: 'app', icon: `icons/apps/${slug}.svg` });
    }
    for (const url of ['https://evilx.com/', 'https://x.com.example.org/', 'https://x.com@evil.example/?next=https://github.com', 'https://example.com/github.com']) {
      expect(S.toolLook({ name: 'browser_nav', args: { url } })).toEqual({ act: 'search' });
    }
  });

  it('не сохраняет сайт после неудачного перехода', () => {
    const { tracker, log } = make();
    tracker.feed({ type: 'activity', botId: 'b', call: { id: 'acp', name: 'mcp.deloist_browser.browser_nav', status: 'running', args: {} } });
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'nav', name: 'browser_nav', args: { action: 'open', url: 'https://x.com' } });
    tracker.feed({ type: 'tool_update', botId: 'b', callId: 'nav', status: 'error' });
    expect(log.at(-2)[1].act).toBe('search');
    tracker.feed({ type: 'tool_start', botId: 'b', callId: 'page', name: 'browser_page', args: {} });
    expect(log.at(-1)[1].act).toBe('search');
    expect(log.some(([k, mood]) => k === 'feel' && mood === 'grumpy')).toBe(true);
  });

  it('начало действия ACP снимает ожидание человека', () => {
    expect(S.clears({ type: 'activity', call: { status: 'running' } })).toBe(true);
    expect(S.clears({ type: 'activity', call: { status: 'ok' } })).toBe(false);
  });

  it('конец хода: настроение по последней ошибке', () => {
    const { tracker } = make();
    tracker.feed({ type: 'error', botId: 'b', message: '429 rate limit' });
    tracker.feed({ type: 'turn_end', botId: 'b', failed: true });
    expect(tracker.failedLast('b')).toBe(true);
    expect(tracker.endMood('b').mood).toBe('sleep');
    expect(tracker.endMood('b').mood).toBe('error'); // причина прочитана один раз
  });

  it('делегат: радость, если у получателя ход удался, и недовольство, если нет', () => {
    const { tracker, log } = make();
    tracker.feed({ type: 'turn_end', botId: 'worker', failed: false });
    tracker.feed({ type: 'delegate_end', botId: 'lead', to: 'worker' });
    tracker.feed({ type: 'turn_end', botId: 'worker2', failed: true });
    tracker.feed({ type: 'delegate_end', botId: 'lead', to: 'worker2' });
    expect(log.filter(([k]) => k === 'feel').map((e) => e[1])).toEqual(['joy', 'grumpy']);
  });

  it('разовые эмоции по событиям без истории', () => {
    const { tracker, log } = make();
    for (const type of ['model_retry', 'model_switched', 'compacting', 'memory_learned', 'skill_learned']) tracker.feed({ type, botId: 'b' });
    tracker.feed({ type: 'approval_closed', botId: 'b', id: 'a', status: 'denied' });
    expect(log.filter(([k]) => k === 'feel').map((e) => e[1])).toEqual(['grumpy', 'surprise', 'dizzy', 'love', 'star', 'sad']);
  });
});

describe('stageLook', () => {
  it('фазы сцены оркестратора -> готовые виды шара', () => {
    expect(S.stageLook('listening')).toEqual({ state: 'idle', act: '' });
    expect(S.stageLook('thinking')).toEqual({ state: 'work', act: 'search' });
    expect(S.stageLook('typing')).toEqual({ state: 'work', act: 'doc' });
    expect(S.stageLook('asking')).toEqual({ state: 'wait', act: '', mood: 'surprise' });
    expect(S.stageLook('done')).toEqual({ state: 'idle', act: '', mood: 'star' });
  });

  it('неизвестная фаза и мусор -> null, без падения на прототипе', () => {
    for (const phase of ['', 'idle', 'toString', '__proto__', null, undefined, 7]) expect(S.stageLook(phase)).toBeNull();
  });
});
