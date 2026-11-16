import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Icon from './Icon.svelte';
import { ICONS } from '../../lib/icons.js';

const SVG_NS = 'http://www.w3.org/2000/svg';
let instance;
let host;

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  vi.restoreAllMocks();
});

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Icon, { target: host, props });
  flushSync();
  return host.querySelector('svg');
}

describe('Icon', () => {
  it('draws the registry markup as real SVG elements with the kit attributes', () => {
    const svg = render({ name: 'search' });
    expect(svg.namespaceURI).toBe(SVG_NS);
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24');
    expect(svg.getAttribute('fill')).toBe('none');
    expect(svg.getAttribute('stroke')).toBe('currentColor');
    expect(svg.getAttribute('stroke-width')).toBe('2');
    expect(svg.getAttribute('stroke-linecap')).toBe('round');
    expect(svg.getAttribute('stroke-linejoin')).toBe('round');
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.getAttribute('width')).toBe('18');
    const children = [...svg.children];
    expect(children.map((node) => node.localName)).toEqual(['circle', 'path']);
    expect(children.every((node) => node.namespaceURI === SVG_NS)).toBe(true);
  });

  it('takes size, stroke and class; size null leaves the size to CSS', () => {
    const svg = render({ name: 'plus', size: null, stroke: 1.8, class: 'k-i' });
    expect(svg.getAttribute('width')).toBeNull();
    expect(svg.getAttribute('stroke-width')).toBe('1.8');
    expect(svg.getAttribute('class')).toBe('k-i');
  });

  it('names the icon for screen readers when a label is given', () => {
    const svg = render({ name: 'close', label: 'Закрыть' });
    expect(svg.getAttribute('role')).toBe('img');
    expect(svg.getAttribute('aria-label')).toBe('Закрыть');
    expect(svg.hasAttribute('aria-hidden')).toBe(false);
  });

  it('draws every registry name', () => {
    for (const name of Object.keys(ICONS)) {
      const svg = render({ name });
      expect(svg.children.length, name).toBeGreaterThan(0);
      unmount(instance);
      host.remove();
      instance = null;
    }
  });

  it('warns and draws nothing for an unknown name', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const svg = render({ name: 'no-such-icon' });
    expect(svg.children.length).toBe(0);
    expect(warn).toHaveBeenCalled();
  });
});
