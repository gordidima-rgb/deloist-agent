import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ProcessCard from './ProcessCard.svelte';

let host;
let instance;
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const item = (over = {}) => ({
  id: 'w1',
  title: 'Отчёт по понедельникам',
  enabled: true,
  updated_at: 1,
  last_run: { status: 'done', at: Date.now() - 120000 },
  card: {
    trigger: 'schedule',
    schedule: { kind: 'daily', time: '09:00', recurrence: { freq: 'weekly', interval: 1, weekdays: [1] } },
    bot_id: 'a',
    task: 'Собери отчёт',
    quiet: true,
    next_run: Date.now() + 86400000,
  },
  ...over,
});

const bots = [{ id: 'a', name: 'Аналитик' }];
const flush = async () => {
  for (let i = 0; i < 6; i += 1) {
    flushSync();
    await new Promise((r) => setTimeout(r, 0));
  }
};

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ProcessCard, { target: host, props: { bots, ...props } });
  flushSync();
}

describe('ProcessCard', () => {
  it('shows when, which bot, what it does, next and last run', () => {
    render({ item: item() });
    expect(host.querySelector('.pc-title').textContent).toBe('Отчёт по понедельникам');
    expect(host.querySelector('.pc-prompt').textContent).toBe('Собери отчёт');
    const tags = [...host.querySelectorAll('.k-tag')].map((t) => t.textContent.trim());
    expect(tags[0]).toBe('Аналитик');
    expect(tags[1]).toContain('09:00');
    expect(host.querySelector('.k-tag--accent')).not.toBeNull();
    expect(host.querySelector('.pc-resume')).toBeNull();
  });

  it('shows the pause reason and offers "Продолжить" after a series of errors', () => {
    const onresume = vi.fn();
    render({ item: item({ error_paused: true, failures: 3, stop_reason: 'login', last_run: { status: 'failed', at: Date.now() } }), onresume });
    expect([...host.querySelectorAll('.k-tag--danger')].pop().textContent).toContain('нужен вход');
    host.querySelector('.pc-resume').click();
    expect(onresume).toHaveBeenCalledWith('w1');
  });

  it('runs, toggles, flips quiet and deletes only on the second click', async () => {
    const onrun = vi.fn();
    const ontoggle = vi.fn();
    const onquiet = vi.fn();
    const ondelete = vi.fn();
    render({ item: item(), onrun, ontoggle, onquiet, ondelete });
    host.querySelector('.pc-run').click();
    expect(onrun).toHaveBeenCalledWith('w1');
    host.querySelector('.pc-quiet').click();
    expect(onquiet).toHaveBeenCalledWith('w1', false);
    host.querySelector('[role="switch"]').click();
    expect(ontoggle).toHaveBeenCalledWith('w1', false);
    host.querySelector('.pc-del').click();
    flushSync();
    expect(ondelete).not.toHaveBeenCalled();
    host.querySelector('.pc-del').click();
    expect(ondelete).toHaveBeenCalledWith('w1');
  });

  it('expands the chain as a list and opens the canvas from it', async () => {
    const graph = {
      nodes: [
        { id: 'b1', kind: 'bot', x: 320, y: 0, fields: { bot_id: 'a', task: 'Собери отчёт' } },
        { id: 't1', kind: 'trigger', x: 0, y: 0, fields: { trigger: 'schedule' }, schedule: { kind: 'daily', time: '09:00' } },
      ],
      edges: [{ id: 'e1', from: 't1', to: 'b1' }],
    };
    const api = vi.fn(async () => ({ workflow: { id: 'w1', graph } }));
    const onopen = vi.fn();
    render({ item: item(), api, onopen });
    expect(host.querySelector('.pc-steps')).toBeNull();
    host.querySelector('.pc-chain-btn').click();
    await flush();
    const rows = [...host.querySelectorAll('.pc-step')].map((r) => r.textContent.replace(/\s+/g, ' ').trim());
    expect(rows).toEqual(['Триггер Каждый день в 09:00', 'Бот Аналитик: Собери отчёт']);
    host.querySelector('.pc-canvas').click();
    expect(onopen).toHaveBeenCalledWith('w1');
  });

  it('has no bot tag and no quiet button for a process without a bot step', () => {
    render({ item: item({ card: { trigger: 'webhook', bot_id: null, task: null, quiet: null, next_run: null } }) });
    expect(host.querySelector('.pc-quiet')).toBeNull();
    expect([...host.querySelectorAll('.k-tag')].map((t) => t.textContent.trim())).toEqual(['Вебхук', expect.any(String)]);
  });
});
