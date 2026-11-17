import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Select from './Select.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('Select', () => {
  it('reports the new value on a native change event', () => {
    const onchange = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Select, {
      target: host,
      props: {
        value: 'a',
        options: [
          { value: 'a', label: 'Первый' },
          { value: 'b', label: 'Второй' },
        ],
        onchange,
      },
    });
    flushSync();

    const select = host.querySelector('select');
    select.value = 'b';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    expect(onchange).toHaveBeenCalledTimes(1);
    expect(select.value).toBe('b');
    expect(host.textContent).toContain('Второй');
  });

  function mountList(props = {}) {
    host = document.createElement('div');
    document.body.append(host);
    instance = mount(Select, {
      target: host,
      props: { value: 'a', options: [{ value: 'a', label: 'Первый' }, { value: 'b', label: 'Второй' }, { value: 'c', label: 'Третий', disabled: true }, { value: 'd', label: 'Четвёртый' }], ...props },
    });
    flushSync();
    return host.querySelector('button[role="combobox"]');
  }
  const key = (el, k) => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); flushSync(); };

  it('opens a custom listbox, moves with arrows over enabled items and picks with Enter', () => {
    const onchange = vi.fn((e) => e.currentTarget.value);
    const button = mountList({ onchange });
    expect(host.querySelector('[role="listbox"]')).toBeNull();
    key(button, 'ArrowDown');
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(host.querySelectorAll('[role="option"]').length).toBe(4);
    key(button, 'ArrowDown');
    key(button, 'ArrowDown');
    expect(button.getAttribute('aria-activedescendant')).toMatch(/-3$/);
    key(button, 'Enter');
    expect(host.querySelector('[role="listbox"]')).toBeNull();
    expect(onchange).toHaveBeenCalledTimes(1);
    expect(onchange.mock.results[0].value).toBe('d');
    expect(button.textContent).toContain('Четвёртый');
  });

  it('closes on Escape and Tab without changing the value, Home and End jump', () => {
    const onchange = vi.fn();
    const button = mountList({ onchange });
    key(button, 'Enter');
    key(button, 'End');
    expect(button.getAttribute('aria-activedescendant')).toMatch(/-3$/);
    key(button, 'Home');
    expect(button.getAttribute('aria-activedescendant')).toMatch(/-0$/);
    key(button, 'Escape');
    expect(host.querySelector('[role="listbox"]')).toBeNull();
    key(button, 'Enter');
    key(button, 'Tab');
    expect(host.querySelector('[role="listbox"]')).toBeNull();
    expect(onchange).not.toHaveBeenCalled();
  });

  it('marks the selected option and ignores clicks on disabled ones', () => {
    const onchange = vi.fn();
    const button = mountList({ onchange });
    button.click();
    flushSync();
    const options = [...host.querySelectorAll('[role="option"]')];
    expect(options[0].getAttribute('aria-selected')).toBe('true');
    options[2].click();
    flushSync();
    expect(onchange).not.toHaveBeenCalled();
    options[1].click();
    flushSync();
    expect(onchange).toHaveBeenCalledTimes(1);
  });
});
