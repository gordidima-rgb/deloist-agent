import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RoutinesPane from './RoutinesPane.svelte';

let host;
let instance;
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

const routine = (over = {}) => ({
  id: 'r1',
  title: 'Новые посты конкурентов в X',
  gist: 'Каждое утро собери новые посты',
  enabled: true,
  quiet: false,
  bot: { name: 'Редактор', orb: '' },
  schedule: 'каждый день в 09:00',
  next: 'ср 09:00',
  last: '11:02',
  lastBad: false,
  ...over,
});

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(RoutinesPane, { target: host, props });
  flushSync();
}

describe('RoutinesPane', () => {
  it('shows title, chips and actions of a routine on the kit', () => {
    render({ routines: [routine()] });
    expect(host.querySelector('.rc-title').textContent).toBe('Новые посты конкурентов в X');
    expect(host.querySelectorAll('.k-tag').length).toBe(4);
    expect(host.querySelector('.k-tag--accent').textContent).toContain('ср 09:00');
    expect(host.querySelector('[role="switch"]').getAttribute('aria-checked')).toBe('true');
  });

  it('clamps a long task and opens it in full by the button, keeping the line breaks', () => {
    const prompt = 'Каждое утро собери новые посты\nи прокомментируй каждый.\n' + 'Правило. '.repeat(40);
    render({ routines: [routine({ prompt })] });
    const text = host.querySelector('.rc-prompt');
    expect(text.textContent).toBe(prompt);
    expect(text.classList.contains('is-clamped')).toBe(true);
    host.querySelector('.rc-more').click();
    flushSync();
    expect(host.querySelector('.rc-prompt').classList.contains('is-open')).toBe(true);
    expect(host.querySelector('.rc-more').getAttribute('aria-expanded')).toBe('true');
    host.querySelector('.rc-more').click();
    flushSync();
    expect(host.querySelector('.rc-prompt').classList.contains('is-clamped')).toBe(true);
  });

  it('a short task is shown whole without the expand button', () => {
    render({ routines: [routine({ prompt: 'Собери новости.' })] });
    expect(host.querySelector('.rc-prompt').classList.contains('is-clamped')).toBe(false);
    expect(host.querySelector('.rc-more')).toBeNull();
  });

  it('takes the lead and empty texts from the props (bot drawer)', () => {
    render({ routines: [], lead: 'Что Редактор делает сам', empty: 'Рутин пока нет.' });
    expect(host.querySelector('.rp-empty').textContent).toBe('Рутин пока нет.');
    expect(host.querySelector('.rp-lead').textContent).toBe('Что Редактор делает сам');
  });

  it('fades a disabled routine and marks a failed last run in red', () => {
    render({ routines: [routine({ enabled: false, lastBad: true })] });
    expect(host.querySelector('.rc').classList.contains('is-off')).toBe(true);
    expect(host.querySelector('.k-tag--danger')).not.toBeNull();
  });

  it('sends the actions with the routine id', () => {
    const onrun = vi.fn();
    const onquiet = vi.fn();
    const ontoggle = vi.fn();
    const ondelete = vi.fn();
    const onnew = vi.fn();
    render({ routines: [routine({ quiet: true })], onrun, onquiet, ontoggle, ondelete, onnew });
    host.querySelector('.rc-run').click();
    host.querySelector('.rc-quiet').click();
    host.querySelector('[role="switch"]').click();
    host.querySelector('.rc-del').click();
    host.querySelector('.rp-new').click();
    expect(onrun).toHaveBeenCalledWith('r1');
    expect(onquiet).toHaveBeenCalledWith('r1', false);
    expect(ontoggle).toHaveBeenCalledWith('r1', false);
    expect(ondelete).toHaveBeenCalledWith('r1');
    expect(onnew).toHaveBeenCalled();
  });

  it('shows the empty note without routines', () => {
    render({ routines: [] });
    expect(host.querySelector('.rp-empty')).not.toBeNull();
    expect(host.querySelector('.rp-new')).not.toBeNull();
  });

  it('a paused routine shows the pause and resumes by its run id', () => {
    const onresume = vi.fn();
    render({ routines: [routine({ paused: { id: 'run_7', label: 'на паузе до 14:00 (лимит)' } })], onresume });
    expect(host.textContent).toContain('на паузе до 14:00 (лимит)');
    host.querySelector('.rc-resume').click();
    expect(onresume).toHaveBeenCalledWith('run_7');
  });

  it('a routine paused by errors shows why and resumes by its own id', () => {
    const onunpause = vi.fn();
    render({ routines: [routine({ errorPaused: { label: 'на паузе: 3 ошибки подряд — нужен вход' } })], onunpause });
    expect(host.textContent).toContain('на паузе: 3 ошибки подряд — нужен вход');
    host.querySelector('.rc-unpause').click();
    expect(onunpause).toHaveBeenCalledWith('r1');
  });

  it('shows no pause controls on a normal routine', () => {
    render({ routines: [routine()] });
    expect(host.querySelector('.rc-unpause')).toBeNull();
    expect(host.querySelector('.rc-resume')).toBeNull();
  });
});
