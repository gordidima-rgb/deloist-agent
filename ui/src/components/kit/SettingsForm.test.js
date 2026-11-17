import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import InfoTip from './InfoTip.svelte';
import RadioGroup from './RadioGroup.svelte';
import FormHarness from './SettingsFormHarness.test.svelte';

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

describe('InfoTip', () => {
  it('is a button named by its label and described by an ink tooltip', () => {
    render(InfoTip, { text: 'Первая строка\nВторая строка', label: 'Что это' });
    const button = host.querySelector('button.k-info__btn');
    const tip = host.querySelector('[role="tooltip"]');
    expect(button.getAttribute('aria-label')).toBe('Что это');
    expect(button.getAttribute('aria-describedby')).toBe(tip.id);
    expect(tip.textContent).toBe('Первая строка\nВторая строка');
    expect(host.querySelector('.k-tip').classList.contains('k-tip--wrap')).toBe(true);
    expect(host.querySelector('.k-tip').classList.contains('k-tip--start')).toBe(true);
  });

  it('draws no tooltip when there is no text', () => {
    render(InfoTip, { text: '', label: 'Пусто' });
    expect(host.querySelector('[role="tooltip"]')).toBeNull();
  });
});

describe('SettingsCard and FieldRow', () => {
  it('draws a card with a title tile and a row whose label is bound to its field', () => {
    render(FormHarness, {});
    expect(host.querySelector('.k-scard__title').textContent).toBe('Section');
    expect(host.querySelector('.k-scard__desc').textContent).toBe('Hint line');
    expect(host.querySelector('.k-scard__head .k-tile svg')).not.toBeNull();
    const label = host.querySelector('.k-frow__label label');
    const field = host.querySelector('.k-frow input');
    expect(label.getAttribute('for')).toBe(field.id);
    expect(host.querySelector('.k-frow .k-tile--sm svg')).not.toBeNull();
    expect(host.querySelector('.k-frow .k-info__btn')).not.toBeNull();
    expect(host.querySelector('.k-frow--block')).not.toBeNull();
  });
});

describe('RadioGroup cards', () => {
  it('shows the title and the hint of each option and selects one', () => {
    render(RadioGroup, {
      variant: 'cards',
      legend: 'Когда спрашивать',
      options: [
        { value: 'a', label: 'Всегда', hint: 'Каждый раз.' },
        { value: 'b', label: 'Никогда', hint: 'Ни разу.' },
      ],
      value: 'b',
    });
    expect(host.querySelector('.k-radio-group').classList.contains('k-radio-group--cards')).toBe(true);
    expect([...host.querySelectorAll('.k-radio__title')].map((el) => el.textContent)).toEqual(['Всегда', 'Никогда']);
    expect([...host.querySelectorAll('.k-radio__hint')].map((el) => el.textContent)).toEqual(['Каждый раз.', 'Ни разу.']);
    expect(host.querySelector('input:checked').value).toBe('b');
  });

  it('keeps the plain list when an option has no hint', () => {
    render(RadioGroup, { options: ['x', 'y'], value: 'x' });
    expect(host.querySelector('.k-radio-group--cards')).toBeNull();
    expect(host.querySelector('.k-radio__title')).toBeNull();
    expect(host.querySelectorAll('.k-radio__label')[1].textContent.trim()).toBe('y');
  });
});
