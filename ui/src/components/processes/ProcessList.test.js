import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ProcessList from './ProcessList.svelte';

// jsdom не умеет Element.animate, а переходы окна журнала на нём держатся.
if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    const animation = { onfinish: null, oncancel: null, cancel() {}, finish() {}, addEventListener() {}, removeEventListener() {} };
    queueMicrotask(() => animation.onfinish?.());
    return animation;
  };
}

let host;
let instance;
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const flush = async () => {
  for (let i = 0; i < 8; i += 1) {
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
  }
};

const row = (id, bot, over = {}) => ({ id, title: `П ${id}`, enabled: true, updated_at: 1, card: { trigger: 'schedule', schedule: { kind: 'daily', time: '09:00' }, bot_id: bot, task: 'Дело', quiet: true }, ...over });

function backend(list, over = {}) {
  const calls = [];
  const api = vi.fn(async (path, opts = {}) => {
    const key = `${opts.method || 'GET'} ${path}`;
    calls.push(key);
    if (over[key]) return over[key](opts);
    if (key === 'GET /api/workflows') return { workflows: list };
    return { ok: true };
  });
  return { api, calls };
}

async function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ProcessList, { target: host, props: { bots: [{ id: 'a', name: 'Аналитик' }, { id: 'b', name: 'Редактор' }], ...props } });
  await flush();
}

const button = (text) => [...host.querySelectorAll('button')].find((b) => b.textContent.includes(text));

describe('ProcessList', () => {
  it('shows a card per process and only the own ones in a bot panel', async () => {
    const { api } = backend([row('w1', 'a'), row('w2', 'b')]);
    await render({ api });
    expect(host.querySelectorAll('.pc').length).toBe(2);
    unmount(instance);
    host.remove();
    await render({ api, botId: 'b' });
    expect([...host.querySelectorAll('.pc-title')].map((t) => t.textContent)).toEqual(['П w2']);
  });

  it('toggles, quiets and resumes through PATCH and runs through POST', async () => {
    const { api, calls } = backend([row('w1', 'a', { error_paused: true, failures: 3 })]);
    await render({ api });
    host.querySelector('[role="switch"]').click();
    await flush();
    host.querySelector('.pc-quiet').click();
    await flush();
    host.querySelector('.pc-resume').click();
    await flush();
    host.querySelector('.pc-run').click();
    await flush();
    const bodies = api.mock.calls.filter((c) => c[1]?.method === 'PATCH').map((c) => c[1].body);
    expect(bodies).toEqual([{ enabled: false }, { quiet: false }, { errorPaused: false }]);
    expect(calls).toContain('POST /api/workflows/w1/run');
  });

  it('makes a process from a phrase and shows the hint when the phrase has no time', async () => {
    let attempt = 0;
    const { api } = backend([], {
      'POST /api/workflows/simple': async () => {
        attempt += 1;
        if (attempt === 1) throw Object.assign(new Error('x'), { status: 400, data: { code: 'no_when' } });
        return { workflow: { id: 'w9', title: 'Собери отчёт' } };
      },
    });
    await render({ api });
    expect(host.textContent).toContain('Процессов пока нет');
    button('Новый процесс').click();
    await flush();
    const input = host.querySelector('input[type="text"]');
    input.value = 'собери отчёт';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    host.querySelector('form').requestSubmit();
    await flush();
    expect(host.textContent).toContain('Не понял, когда запускать');
    host.querySelector('form').requestSubmit();
    await flush();
    expect(host.querySelector('form')).toBeNull();
    const posts = api.mock.calls.filter((c) => c[0] === '/api/workflows/simple');
    expect(posts[0][1].body).toEqual({ phrase: 'собери отчёт', bot_id: 'a' });
  });

  it('opens a blank canvas under the first free name', async () => {
    const onopen = vi.fn();
    const { api } = backend([row('w1', 'a', { title: 'Новый процесс' })], { 'POST /api/workflows': async () => ({ workflow: { id: 'w7' } }) });
    await render({ api, onopen });
    button('Новый процесс').click();
    await flush();
    button('Пустой холст').click();
    await flush();
    const post = api.mock.calls.find((c) => c[0] === '/api/workflows' && c[1]?.method === 'POST');
    expect(post[1].body.title).toBe('Новый процесс 2');
    expect(onopen).toHaveBeenCalledWith('w7');
  });

  it('offers "continue now" for a run paused on a limit and resumes it', async () => {
    const until = Date.now() + 3600e3;
    const { api, calls } = backend([row('w1', 'a'), row('w2', 'b')], {
      'GET /api/needs': async () => ({ items: [{ kind: 'run', paused: true, id: 'run9', routineId: 'w1', reason: 'limit', until }] }),
    });
    await render({ api });
    const cards = host.querySelectorAll('.pc');
    expect(cards[0].textContent).toContain('на паузе до');
    expect(cards[0].textContent).toContain('лимит');
    expect(cards[1].textContent).not.toContain('на паузе до');
    [...cards[0].querySelectorAll('button')].find((b) => b.textContent.includes('Продолжить сейчас')).click();
    await flush();
    expect(calls).toContain('POST /api/runs/run9/resume');
  });

  it('has the journal and the calendar download in the header, but not in a bot panel', async () => {
    const { api } = backend([row('w1', 'a')], {
      'GET /api/journal?limit=200': async () => ({ entries: [{ id: 'j1', at: Date.now(), botName: 'Аналитик', title: 'Отчёт', status: 'needs', text: 'нужен ответ', missed: ['Что делать?'], routineId: 'w1' }, { id: 'j2', at: Date.now(), botName: 'Аналитик', title: 'Проверка', status: 'quiet' }], usage: { tokens: 1200 }, limit: 5000 }),
    });
    await render({ api });
    expect(host.querySelector('a[href="/api/tasks.ics"]')?.hasAttribute('download')).toBe(true);
    button('Журнал').click();
    await flush();
    const text = document.body.textContent;
    expect(text).toContain('Сегодня потрачено');
    expect(text).toContain('Что делать?');
    expect(text).toContain('Тихих проверок');
    [...document.querySelectorAll('button')].find((b) => b.textContent.includes('Запустить снова')).click();
    await flush();
    expect(api.mock.calls.some((c) => c[0] === '/api/workflows/w1/run')).toBe(true);
    unmount(instance);
    host.remove();
    await render({ api, botId: 'a' });
    expect(button('Журнал')).toBeUndefined();
    expect(host.querySelector('a[href="/api/tasks.ics"]')).toBeNull();
  });
});
