import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, tick, unmount } from 'svelte';
import WhatsNew from './WhatsNew.svelte';
import UpdatePlaque from './UpdatePlaque.svelte';

if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    const animation = { onfinish: null, oncancel: null, cancel() {}, finish() {}, addEventListener() {}, removeEventListener() {} };
    queueMicrotask(() => animation.onfinish?.());
    return animation;
  };
}

const SECTIONS = [
  { version: '0.4.0', date: '2026-11-17', markdown: '### Добавлено\n\n- **Окно.** Новое окно <b>x</b>\n\n### Изменено\n\n- **Быстрее.** Ускорено\n\n### Исправлено\n\n- Починено `вход`' },
];

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  delete window.CrewAvatar;
});

async function show(Component, props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Component, { target: host, props });
  flushSync();
  await tick();
  return document.body;
}

const text = (selector) => document.body.querySelector(selector)?.textContent?.replace(/\s+/g, ' ').trim();
const click = async (el) => {
  el.click();
  flushSync();
  await tick();
};
const button = (name) => [...document.body.querySelectorAll('button')].find((b) => b.textContent.trim() === name || b.getAttribute('aria-label') === name);

describe('WhatsNew', () => {
  it('shows one page per category and ends with «Понятно»', async () => {
    const onclose = vi.fn();
    await show(WhatsNew, { current: '0.4.0', from: '0.3.7', sections: SECTIONS, onclose });
    expect(text('.k-modal__title')).toContain('0.4.0');
    expect(text('.wn-label')).toBe('Новое');
    expect(text('.wn-count')).toBe('1 из 3');
    expect(text('.wn-lead')).toBe('Окно.');
    // HTML из заметок не проходит.
    expect(document.body.querySelector('.wn-item b')).toBeNull();
    await click(button('Далее'));
    expect(text('.wn-label')).toBe('Улучшено');
    await click(button('Далее'));
    expect(text('.wn-label')).toBe('Исправлено');
    expect(document.body.querySelector('.wn-item code')?.textContent).toBe('вход');
    expect(button('Далее')).toBeUndefined();
    await click(button('Понятно'));
    expect(onclose).toHaveBeenCalledOnce();
  });

  it('turns pages with the arrow keys and the pager buttons', async () => {
    await show(WhatsNew, { current: '0.4.0', sections: SECTIONS });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    flushSync();
    await tick();
    expect(text('.wn-label')).toBe('Улучшено');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    flushSync();
    await tick();
    expect(text('.wn-label')).toBe('Новое');
    expect(button('Назад').disabled).toBe(true);
  });

  it('closes with Escape and with the cross', async () => {
    const onclose = vi.fn();
    await show(WhatsNew, { current: '0.4.0', sections: SECTIONS, onclose });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    flushSync();
    expect(onclose).toHaveBeenCalledOnce();
  });

  it('opens the full list through loadAll and goes back', async () => {
    const loadAll = vi.fn(async () => [...SECTIONS, { version: '0.3.7', date: '2026-11-16', markdown: '### Добавлено\n\n- **Старое.** давно' }]);
    await show(WhatsNew, { current: '0.4.0', sections: SECTIONS, loadAll });
    await click(button('Все изменения'));
    await vi.waitFor(() => expect(document.body.querySelectorAll('.wn-release').length).toBe(2));
    expect(text('.wn-release__version')).toBe('0.4.0');
    await click(button('Назад'));
    expect(document.body.querySelector('.wn-page')).not.toBeNull();
  });

  it('hides the full-list link without a loader and the pager with one page', async () => {
    await show(WhatsNew, { current: '0.4.0', sections: [{ version: '0.4.0', markdown: '- один' }] });
    expect(button('Все изменения')).toBeUndefined();
    expect(document.body.querySelector('.wn-pager')).toBeNull();
    expect(button('Понятно')).toBeDefined();
  });

  it('shows the section image when there is one and falls back to orbs when it breaks', async () => {
    await show(WhatsNew, { current: '1', sections: [{ version: '1', markdown: '![x](docs/images/whats-new/a.webp)\n\n- p' }] });
    const img = document.body.querySelector('.wn-image');
    expect(img.getAttribute('src')).toBe('/api/whats-new/image/a.webp');
    img.dispatchEvent(new Event('error'));
    flushSync();
    await tick();
    expect(document.body.querySelector('.wn-image')).toBeNull();
    expect(document.body.querySelector('.wn-paper')).not.toBeNull();
  });

  it('mounts live orbs when the avatar library is there', async () => {
    const feel = vi.fn();
    const mountOrb = vi.fn(() => ({ feel, destroy() {} }));
    window.CrewAvatar = { ORBS: new Array(14).fill({}), COLORS: ['#fff'], mount: mountOrb };
    await show(WhatsNew, { current: '1', sections: SECTIONS });
    expect(mountOrb).toHaveBeenCalledTimes(5);
    expect(document.body.querySelector('.wn-paper')).toBeNull();
  });
});

describe('UpdatePlaque', () => {
  it('offers the notes and the restart', async () => {
    const onrestart = vi.fn();
    const onwhatsnew = vi.fn();
    await show(UpdatePlaque, { version: '0.4.1', onrestart, onwhatsnew });
    expect(text('.up-title')).toBe('Deloist 0.4.1 готов');
    await click(button('Что нового'));
    await click(button('Перезапустить'));
    expect(onwhatsnew).toHaveBeenCalledOnce();
    expect(onrestart).toHaveBeenCalledOnce();
  });

  it('waits for the bots without letting the button fire twice', async () => {
    await show(UpdatePlaque, { version: '0.4.1', pending: true });
    expect(button('Перезапустить').disabled).toBe(true);
    expect(text('.up-note')).toContain('боты закончат');
  });
});
