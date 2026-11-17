import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, tick, unmount } from 'svelte';
import Modal from './Modal.svelte';

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
let trigger;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  trigger?.remove();
  instance = null;
  host = null;
  trigger = null;
});

function open() {
  trigger = document.createElement('button');
  trigger.textContent = 'Открыть';
  document.body.append(trigger);
  trigger.focus();

  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Modal, { target: host, props: { open: true, title: 'Настройки', onclose } });
  flushSync();
  return { onclose };
}

const onclose = vi.fn();

describe('Modal', () => {
  it('closes on Escape and returns focus to the trigger', async () => {
    onclose.mockClear();
    open();
    await tick();
    flushSync();
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
    expect(host.contains(document.activeElement)).toBe(true);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    flushSync();

    expect(onclose).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(trigger);
    await vi.waitFor(() => expect(host.querySelector('[role="dialog"]')).toBeNull());
  });

  it('keeps Tab inside the dialog', async () => {
    onclose.mockClear();
    open();
    await tick();
    flushSync();
    const dialog = host.querySelector('[role="dialog"]');
    const buttons = [...dialog.querySelectorAll('button')];
    const last = buttons[buttons.length - 1];
    last.focus();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    flushSync();

    expect(document.activeElement).toBe(buttons[0]);
  });
});
