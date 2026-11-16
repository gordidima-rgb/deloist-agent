import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ApprovalCard from './ApprovalCard.svelte';

let instance;
let host;

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ApprovalCard, { target: host, props });
  flushSync();
  return host;
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = host = null;
});

const actions = [
  { answer: 'once', label: 'Разрешить один раз' },
  { answer: 'always', label: 'Всегда разрешать здесь' },
  { answer: 'deny', label: 'Запретить' },
];

describe('ApprovalCard', () => {
  it('shows a plain sentence, the risk, the bot words as text and the raw command collapsed', () => {
    const el = render({
      title: 'Редактор хочет изменить файл «a.md» в своей папке',
      risk: 'low',
      riskLabel: 'Риск: низкий',
      purpose: '<img src=x onerror=alert(1)> обновить обзор',
      purposeLabel: 'Бот объясняет:',
      details: "pwsh -Command '$p = 1'",
      detailsLabel: 'Показать технические детали',
      actions,
    });
    expect(el.querySelector('.k-approval__title').textContent).toContain('Редактор хочет изменить файл');
    expect(el.querySelector('.k-approval__risk').dataset.risk).toBe('low');
    const purpose = el.querySelector('.k-approval__purpose');
    expect(purpose.textContent).toContain('Бот объясняет:');
    expect(purpose.querySelector('img')).toBeNull();
    expect(purpose.textContent).toContain('<img src=x');
    const details = el.querySelector('details.k-approval__details');
    expect(details.open).toBe(false);
    expect(details.querySelector('pre').textContent).toContain('$p = 1');
    expect(el.querySelector('[data-answer="once"]')).not.toBeNull();
  });

  it('shows the question, what will happen and the scope', () => {
    const el = render({ title: 'Нужно разрешение: платная картинка', what: 'нарисуй кота', scope: 'Где: везде', kind: 'оплата', actions });
    expect(el.querySelector('.k-approval__title').textContent).toBe('Нужно разрешение: платная картинка');
    expect(el.querySelector('.k-approval__what').textContent).toBe('нарисуй кота');
    expect(el.querySelector('.k-approval__scope').textContent).toContain('оплата');
  });

  it('reports the answer of the clicked button', () => {
    const onanswer = vi.fn();
    const el = render({ title: 'T', actions, onanswer });
    el.querySelector('[data-answer="once"]').click();
    el.querySelector('[data-answer="deny"]').click();
    expect(onanswer.mock.calls).toEqual([['once'], ['deny']]);
  });

  it('makes the first action primary, or attention when irreversible, and deny ghost', () => {
    let el = render({ title: 'T', actions });
    expect(el.querySelector('[data-answer="once"]').className).toContain('k-btn--primary');
    expect(el.querySelector('[data-answer="deny"]').className).toContain('k-btn--ghost');
    unmount(instance);
    host.remove();
    el = render({ title: 'T', actions, danger: true });
    expect(el.querySelector('[data-answer="once"]').className).toContain('k-btn--attention');
  });

  it('disables the buttons while busy', () => {
    const el = render({ title: 'T', actions, busy: true });
    expect([...el.querySelectorAll('button')].every((b) => b.disabled)).toBe(true);
  });

  it('replaces the buttons with the decision once resolved', () => {
    const el = render({ title: 'T', actions, resolved: 'разрешено' });
    expect(el.querySelectorAll('button').length).toBe(0);
    expect(el.querySelector('.k-approval__resolved').textContent).toBe('разрешено');
  });
});
