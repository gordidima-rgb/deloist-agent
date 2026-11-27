import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ProcessList from './ProcessList.svelte';

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
    expect(bodies).toEqual([{ enabled: false }, { quiet: false }, { error_paused: false }]);
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
});
