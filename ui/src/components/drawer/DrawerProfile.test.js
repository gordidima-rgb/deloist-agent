import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, tick } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import DrawerProfile from './DrawerProfile.svelte';

// jsdom не умеет Element.animate, а Svelte-переходы окна на нём держатся.
if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    const animation = { onfinish: null, oncancel: null, cancel() {}, finish() {}, addEventListener() {}, removeEventListener() {} };
    queueMicrotask(() => animation.onfinish?.());
    return animation;
  };
}

let handle;
let host;

function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(DrawerProfile, host, { skills: [], levels: {}, onsave: vi.fn(async () => true), ...props });
}

const waitFor = (check) =>
  vi.waitFor(
    () => {
      flushSync();
      check();
    },
    { timeout: 3000 },
  );

const slider = () => host.querySelector('.dp-compact input[type="range"]');
// Пояснения к памяти лежат в подсказке «i» внутри врезки.
const info = () => host.querySelector('.dp-compact .k-tooltip').textContent.replace(/\s+/g, ' ');
const field = (label) => [...host.querySelectorAll('.k-frow')].find((row) => row.querySelector('label')?.textContent.trim() === label);
const input = (label) => field(label).querySelector('input, select, textarea');
const buttonByLabel = (label) => host.querySelector(`button[aria-label="${label}"]`);
const million = { known: true, window: 1000000, usable: 870000, min: 20, max: 95, step: 5, percent: 20, stored: false, currentTokens: 40000, mode: 'normal' };

function typeValue(el, value) {
  el.value = String(value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

afterEach(() => {
  handle?.destroy();
  host?.remove();
  document.body.querySelectorAll('.k-modal').forEach((el) => el.remove());
  handle = null;
  host = null;
});

describe('DrawerProfile compaction slider', () => {
  it('shows the slider with the tokens of the real window inside the tooltip', async () => {
    mount({ bot: { id: 'b1', name: 'Кодер', compactAtPercent: 60 }, contextInfo: async () => ({ ...million, stored: true, percent: 60, currentTokens: 600000 }) });
    await waitFor(() => expect(info()).toContain('600'));
    expect(slider()).not.toBeNull();
    expect(host.querySelector('.dp-compact .k-label').textContent).toBe('Сжимать при 60% лимита');
    expect(info()).toContain('≈ 600 000 из 1 000 000 токенов');
    expect(slider().getAttribute('aria-valuetext')).toBe('60% лимита');
    expect(host.textContent).not.toContain('Большая (для программирования)');
  });

  it('puts the ticks at the real minimum, the two marks and the maximum', async () => {
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 60 }, contextInfo: async () => ({ ...million, stored: true, percent: 60 }) });
    await waitFor(() => expect(slider()).not.toBeNull());
    const marks = [...host.querySelectorAll('.dp-compact .k-slider__mark')].map((el) => el.textContent.trim());
    expect(marks).toEqual(['20', '50', '80', '95']);
  });

  it('warns about cost above 200k tokens and not below', async () => {
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 60 }, contextInfo: async () => ({ ...million, stored: true, percent: 60, currentTokens: 600000 }) });
    await waitFor(() => expect(info()).toContain('600'));
    expect(host.querySelector('.dp-compact__warn')).not.toBeNull();
    typeValue(slider(), 20);
    expect(info()).toContain('≈ 200 000 из 1 000 000');
    expect(host.querySelector('.dp-compact__warn')).toBeNull();
  });

  it('caps the shown tokens at the usable part of the window', async () => {
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 95 }, contextInfo: async () => ({ ...million, stored: true, percent: 95 }) });
    await waitFor(() => expect(info()).toContain('Выше 870 000 токенов порог не поднимается'));
  });

  it('says so when the window is unknown and counts from the safe default', async () => {
    const unknown = { known: false, window: null, usable: 32000, min: 20, max: 95, step: 5, percent: 60, stored: true, currentTokens: 24000, mode: 'normal' };
    mount({ bot: { id: 'b1', name: 'К', compactAtPercent: 60 }, contextInfo: async () => unknown });
    await waitFor(() => expect(info()).toContain('неизвестен'));
    expect(info()).toContain('от 40 000 токенов: ≈ 24 000');
  });

  it('keeps the old default untouched until the slider moves and then sends the percent', async () => {
    const onsave = vi.fn(async () => true);
    mount({ bot: { id: 'b1', name: 'К' }, contextInfo: async () => million, onsave });
    await waitFor(() => expect(info()).toContain('Пока действует прежний режим: ≈ 40 000'));
    expect(onsave).not.toHaveBeenCalled();

    // Другая правка уходит сама и порог не трогает.
    const effort = input('Усилие');
    effort.value = 'high';
    effort.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
    expect(onsave.mock.calls[0][0]).not.toHaveProperty('compactAtPercent');
    expect(onsave.mock.calls[0][0]).not.toHaveProperty('contextMode');
    expect(onsave.mock.calls[0][0].effort).toBe('high');

    typeValue(slider(), 45);
    slider().dispatchEvent(new Event('change', { bubbles: true }));
    expect(host.querySelector('.dp-compact .k-label').textContent).toBe('Сжимать при 45% лимита');
    expect(info()).toContain('≈ 450 000 из 1 000 000');
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(2));
    expect(onsave.mock.calls[1][0].compactAtPercent).toBe(45);
  });

  it('starts the slider where the old mode keeps it', async () => {
    mount({ bot: { id: 'b1', name: 'К', contextMode: 'large' }, contextInfo: async () => ({ ...million, percent: 50, currentTokens: 500000, mode: 'large' }) });
    await waitFor(() => expect(slider().value).toBe('50'));
  });
});

describe('DrawerProfile autosave', () => {
  it('has a header with the status line and no Save or Cancel button', () => {
    mount({ bot: { id: 'b1', name: 'Редактор' }, folder: 'data/workspaces/b1' });
    expect(host.querySelector('.dp-head__title').textContent).toBe('Настройки бота');
    expect(host.querySelector('.dp-status').getAttribute('aria-live')).toBe('polite');
    expect(host.querySelector('.dp-status').textContent).toContain('Изменения сохраняются сами');
    const labels = [...host.querySelectorAll('button')].map((b) => b.textContent.trim());
    expect(labels).not.toContain('Сохранить');
    expect(labels).not.toContain('Отмена');
    expect(host.textContent).not.toContain('Удаление');
    expect(host.querySelector('.dp-folder code').textContent).toBe('data/workspaces/b1');
    expect(buttonByLabel('Удалить бота')).not.toBeNull();
  });

  it('saves typed text after a pause, not on every key, and reports the status', async () => {
    const onsave = vi.fn(async () => true);
    mount({ bot: { id: 'b1', name: 'Редактор', role: 'Пишет' }, onsave });
    typeValue(input('Чем занимается'), 'Пишет тексты');
    typeValue(input('Чем занимается'), 'Пишет тексты твоим голосом');
    expect(onsave).not.toHaveBeenCalled();
    expect(host.querySelector('.dp-status').textContent).toContain('Сохраняю…');
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
    expect(onsave.mock.calls[0][0].role).toBe('Пишет тексты твоим голосом');
    expect(onsave.mock.calls[0][0].name).toBe('Редактор');
    await waitFor(() => expect(host.querySelector('.dp-status').textContent).toContain('Сохранено'));
  });

  it('does not save an empty name', async () => {
    const onsave = vi.fn(async () => true);
    mount({ bot: { id: 'b1', name: 'Редактор' }, onsave });
    typeValue(input('Имя'), '   ');
    typeValue(input('Чем занимается'), 'Новая роль');
    await waitFor(() => expect(onsave).toHaveBeenCalled());
    for (const [body] of onsave.mock.calls) expect(body).not.toHaveProperty('name');
  });

  it('writes pending text when the panel is closed before the pause ends', async () => {
    const onsave = vi.fn(async () => true);
    mount({ bot: { id: 'b1', name: 'Редактор' }, onsave });
    typeValue(input('Инструкции'), 'Пиши коротко.');
    expect(onsave).not.toHaveBeenCalled();
    handle.destroy();
    handle = null;
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
    expect(onsave.mock.calls[0][0].persona).toBe('Пиши коротко.');
  });

  it('shows an error in the header when the save fails and retries on the next change', async () => {
    const onsave = vi.fn(async () => false);
    mount({ bot: { id: 'b1', name: 'Редактор' }, onsave });
    typeValue(input('Чем занимается'), 'А');
    await waitFor(() => expect(host.querySelector('.dp-status').textContent).toContain('Не удалось сохранить'));
    typeValue(input('Чем занимается'), 'Аб');
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(2));
  });

  it('saves a chosen look as soon as the picker reports it', async () => {
    const onsave = vi.fn(async () => true);
    let pick;
    mount({ bot: { id: 'b1', name: 'К' }, onsave, mountLook: (_el, onpick) => (pick = onpick) });
    await tick();
    pick(3);
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
  });
});

describe('DrawerProfile form', () => {
  it('counts the instruction characters out of 12 000 and warns near the limit', () => {
    mount({ bot: { id: 'b1', name: 'К', persona: 'abc' } });
    expect(host.querySelector('.dp-count').textContent.replace(/\s/g, ' ')).toBe('3 / 12 000');
    expect(host.querySelector('.dp-count').classList.contains('is-over')).toBe(false);
    typeValue(input('Инструкции'), 'x'.repeat(11000));
    expect(host.querySelector('.dp-count').classList.contains('is-over')).toBe(true);
    expect(host.querySelector('textarea').getAttribute('maxlength')).toBe('12000');
  });

  it('keeps explanations in tooltips that open on focus', () => {
    mount({ bot: { id: 'b1', name: 'К' } });
    flushSync();
    const row = field('Инструкции');
    const tip = row.querySelector('[role="tooltip"]');
    expect(tip.textContent).toContain('Бот читает это перед каждым ответом');
    const btn = row.querySelector('.k-info__btn');
    expect(btn.getAttribute('aria-describedby')).toBe(tip.id);
    expect(btn.getAttribute('aria-label')).toBe('Что писать в инструкциях');
  });

  it('renames the fields in plain words', () => {
    mount({ bot: { id: 'b1', name: 'К' } });
    expect(field('Чем занимается')).toBeDefined();
    expect(field('Инструкции')).toBeDefined();
    expect(host.textContent).not.toContain('Характер и правила');
    expect(host.textContent).not.toContain('Журнал разрешений');
    expect(host.textContent).toContain('Что бот делал');
    expect(host.querySelector('#pf-activity')).not.toBeNull();
  });

  it('asks about permissions with three radio cards and saves the choice at once', async () => {
    const onsave = vi.fn(async () => true);
    mount({ bot: { id: 'b1', name: 'К', approval: 'ask' }, onsave });
    const radios = [...host.querySelectorAll('.k-radio-group--cards input[type="radio"]')];
    expect(radios.map((r) => r.value)).toEqual(['strict', 'ask', 'auto']);
    expect(radios.find((r) => r.checked).value).toBe('ask');
    expect(host.querySelector('.k-radio-group--cards legend').textContent).toBe('Когда бот спрашивает тебя');
    expect(host.textContent).toContain('Только за пределами своей папки');
    const auto = radios[2];
    auto.checked = true;
    auto.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => expect(onsave).toHaveBeenCalledTimes(1));
    expect(onsave.mock.calls[0][0].approval).toBe('auto');
  });

  it('opens the sites list inline and revokes a site', async () => {
    const onrevokesite = vi.fn();
    mount({ bot: { id: 'b1', name: 'К', browserSites: ['example.com'] }, onrevokesite });
    const panel = host.querySelector('#dp-sites');
    const row = panel.closest('.k-lrow');
    expect(row.classList.contains('is-open')).toBe(false);
    row.querySelector('button.k-lrow__main').click();
    flushSync();
    expect(row.classList.contains('is-open')).toBe(true);
    panel.querySelector('.dp-x').click();
    expect(onrevokesite).toHaveBeenCalledWith('example.com');
  });

  it('opens the browser for the Google sign-in from its row', () => {
    const ongoogle = vi.fn();
    mount({ bot: { id: 'b1', name: 'К' }, ongoogle });
    const row = host.querySelector('.dp-google');
    expect(row.textContent).toContain('Бот не вошёл');
    expect(row.textContent).toContain('Войти');
    row.querySelector('.k-btn').click();
    expect(ongoogle).toHaveBeenCalled();
  });
});

describe('DrawerProfile delete', () => {
  const dialog = () => document.body.querySelector('[role="dialog"]');

  it('asks in a kit modal, focuses Cancel and deletes only after Delete', async () => {
    const ondelete = vi.fn(async () => {});
    mount({ bot: { id: 'b1', name: 'Редактор' }, ondelete });
    buttonByLabel('Удалить бота').click();
    await tick();
    flushSync();
    expect(dialog().querySelector('h2').textContent).toBe('Удалить бота «Редактор»?');
    expect(dialog().textContent).toContain('Вы уверены, что хотите удалить бота и всю его историю?');
    expect(dialog().textContent).toContain('Папка с файлами бота останется на диске');
    await tick();
    expect(document.activeElement.textContent.trim()).toBe('Отмена');
    expect(ondelete).not.toHaveBeenCalled();
    const del = [...dialog().querySelectorAll('button')].find((b) => b.textContent.trim() === 'Удалить');
    expect(del.classList.contains('k-btn--attention')).toBe(true);
    del.click();
    await waitFor(() => expect(ondelete).toHaveBeenCalledTimes(1));
  });

  it('closes with Escape and does not delete', async () => {
    const ondelete = vi.fn();
    mount({ bot: { id: 'b1', name: 'К' }, ondelete });
    buttonByLabel('Удалить бота').click();
    await tick();
    flushSync();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await waitFor(() => expect(dialog()).toBeNull());
    expect(ondelete).not.toHaveBeenCalled();
  });
});
