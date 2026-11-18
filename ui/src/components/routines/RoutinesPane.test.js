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
    host.querySelector('.rt-run').click();
    host.querySelector('.rt-quiet').click();
    host.querySelector('[role="switch"]').click();
    host.querySelector('.rt-del').click();
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

  it('shows no pause controls on a normal routine', () => {
    render({ routines: [routine()] });
    expect(host.querySelector('.rc-resume')).toBeNull();
  });
});
