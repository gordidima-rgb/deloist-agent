import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Slider from './Slider.svelte';

let instance;
let host;

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Slider, { target: host, props });
  flushSync();
  return host.querySelector('input[type="range"]');
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('Slider', () => {
  it('is a native range with the given bounds, step and spoken value', () => {
    const input = render({ value: 60, min: 20, max: 95, step: 5, label: 'Сжимать при 60% лимита', valueText: '60% лимита' });
    expect(input.min).toBe('20');
    expect(input.max).toBe('95');
    expect(input.step).toBe('5');
    expect(input.value).toBe('60');
    expect(input.getAttribute('aria-valuetext')).toBe('60% лимита');
    expect(host.querySelector('.k-label').textContent).toBe('Сжимать при 60% лимита');
  });

  it('reports input events and clamps to the bounds', () => {
    const oninput = vi.fn();
    const input = render({ value: 60, min: 20, max: 95, step: 5, oninput });
    input.value = '75';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(oninput).toHaveBeenCalledTimes(1);
    input.value = '500';
    expect(input.value).toBe('95');
    input.value = '1';
    expect(input.value).toBe('20');
  });

  it('draws marks at their share of the track', () => {
    render({ value: 60, min: 20, max: 95, step: 5, marks: [50, 80] });
    const marks = [...host.querySelectorAll('.k-slider__mark')];
    expect(marks.map((m) => m.textContent)).toEqual(['50', '80']);
    expect(marks[0].getAttribute('style')).toContain('--k-mark: 0.4');
    expect(host.querySelector('.k-slider__marks').getAttribute('aria-hidden')).toBe('true');
  });

  it('shows the plain value when asked and no text is given', () => {
    render({ value: 30, label: 'Яркость', showValue: true });
    expect(host.querySelector('.k-slider__value').textContent).toBe('30');
  });
});
