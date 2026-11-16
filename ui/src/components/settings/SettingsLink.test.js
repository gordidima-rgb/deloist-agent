import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import SettingsLink from './SettingsLink.svelte';
import SettingsHint from './SettingsHint.svelte';

let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

function render(Component, props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Component, { target: host, props });
  flushSync();
}

describe('SettingsLink', () => {
  it('shows the label, the sublabel and the current value as text', () => {
    render(SettingsLink, { id: 'skills', label: 'Навыки', sublabel: 'Правила для задач', value: '5' });
    const row = host.querySelector('.k-row.s-link');
    expect(row.tagName).toBe('BUTTON');
    expect(row.dataset.link).toBe('skills');
    expect(row.querySelector('.k-row__label').textContent).toBe('Навыки');
    expect(row.querySelector('.k-row__sub').textContent).toBe('Правила для задач');
    expect(row.querySelector('.s-link__value').textContent).toBe('5');
  });

  it('omits the value and the sublabel when they are empty and still has the chevron', () => {
    render(SettingsLink, { label: 'MCP-серверы' });
    expect(host.querySelector('.s-link__value')).toBeNull();
    expect(host.querySelector('.k-row__sub')).toBeNull();
    expect(host.querySelector('.s-link__chev')).not.toBeNull();
  });

  it('calls onclick when pressed', () => {
    const onclick = vi.fn();
    render(SettingsLink, { label: 'Картинки', onclick });
    host.querySelector('button').click();
    expect(onclick).toHaveBeenCalledTimes(1);
  });
});

describe('SettingsHint', () => {
  it('renders a question mark button described by the tooltip text', () => {
    render(SettingsHint, { text: 'Длинное пояснение' });
    const button = host.querySelector('button.s-hint');
    expect(button.textContent).toBe('?');
    const tip = host.querySelector('[role="tooltip"]');
    expect(tip.textContent).toBe('Длинное пояснение');
    expect(button.getAttribute('aria-describedby')).toBe(tip.id);
  });
});
