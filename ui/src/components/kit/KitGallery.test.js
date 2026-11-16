import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, tick, unmount } from 'svelte';
import KitGallery from './KitGallery.svelte';

// jsdom не умеет Element.animate, а Svelte-переходы окна на нём держатся.
if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    const animation = {
      onfinish: null,
      oncancel: null,
      cancel() {},
      finish() {},
      addEventListener() {},
      removeEventListener() {},
    };
    queueMicrotask(() => animation.onfinish?.());
    return animation;
  };
}

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('KitGallery', () => {
  it('renders the component sections in both themes', () => {
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(KitGallery, { target: host });
    flushSync();

    expect(host.querySelectorAll('[data-theme="light"]').length).toBeGreaterThan(5);
    expect(host.querySelectorAll('[data-theme="dark"]').length).toBeGreaterThan(5);
    expect(host.textContent).toContain('Кнопки');
    expect(host.querySelectorAll('.k-btn').length).toBeGreaterThan(5);
    expect(host.querySelectorAll('.k-section').length).toBe(2);
  });

  it('opens a dialog with footer actions from the gallery', async () => {
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(KitGallery, { target: host });
    flushSync();

    const button = [...host.querySelectorAll('button')].find((el) => el.textContent.includes('Открыть окно'));
    button.click();
    flushSync();
    await tick();
    flushSync();

    const dialog = document.querySelector('.k-modal [role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog.querySelector('.k-modal__foot')).not.toBeNull();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    flushSync();
  });
});
