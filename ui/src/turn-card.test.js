import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// turn-card.js — обычный скрипт страницы: кладёт всё в window.CrewTurnCard.
const source = readFileSync(resolve(process.cwd(), '../web/turn-card.js'), 'utf8');

const labels = {
  thinking: 'Думает', choosing: 'Выбираю, кто ответит', stop: 'Стоп', stopHint: 'Остановить',
  failed: 'Не отправилось', sec: '{n} с', min: '{m} мин {n} с',
};
const bot = { id: 'b1', name: 'Мира' };

let host;
let avatars;
let stops;
let card;

beforeEach(() => {
  vi.useFakeTimers();
  // eslint-disable-next-line no-new-func
  new Function(source).call(window);
  host = document.createElement('div');
  document.body.append(host);
  avatars = [];
  stops = 0;
  card = window.CrewTurnCard.create({
    root: () => host,
    labels,
    now: () => Date.now(),
    mountAvatar: (el, who) => {
      const handle = { who, destroyed: false, destroy() { this.destroyed = true; } };
      avatars.push(handle);
      el.textContent = 'orb';
      return handle;
    },
    onStop: () => { stops += 1; },
  });
});

afterEach(() => {
  card.detach();
  host.remove();
  vi.useRealTimers();
});

const cards = () => host.querySelectorAll('.turn-card');
const line = () => host.querySelector('.tc-line').textContent;

describe('turn card', () => {
  it('shows at once on send with a zero timer, avatar, name and Stop', () => {
    card.begin({ key: 'bot:b1', bot });
    expect(cards()).toHaveLength(1);
    expect(host.querySelector('.tc-name').textContent).toBe('Мира');
    expect(host.querySelector('.tc-orb').textContent).toBe('orb');
    expect(line()).toBe('Думает…0 с');
    expect(host.querySelector('.tc-stop')).not.toBeNull();
  });

  it('ticks every second and never announces the time', () => {
    card.begin({ key: 'bot:b1', bot });
    vi.advanceTimersByTime(3000);
    expect(host.querySelector('.tc-time').textContent).toBe('3 с');
    expect(host.querySelector('.tc-time').getAttribute('aria-hidden')).toBe('true');
    const status = host.querySelector('.tc-status');
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.textContent).toBe('Думает…');
    vi.advanceTimersByTime(61_000);
    expect(host.querySelector('.tc-time').textContent).toBe('1 мин 4 с');
  });

  it('is the same block when the real events arrive: no duplicate, no timer reset', () => {
    card.begin({ key: 'bot:b1', bot });
    const first = host.querySelector('.turn-card');
    vi.advanceTimersByTime(6000);
    // turn_start приходит через 6 секунд: тот же вызов, что и из обработчика события
    expect(card.begin({ key: 'bot:b1', bot }).created).toBe(false);
    card.phase('thinking');
    card.status('Читает файл');
    expect(cards()).toHaveLength(1);
    expect(host.querySelector('.turn-card')).toBe(first);
    expect(host.querySelector('.tc-time').textContent).toBe('6 с');
    expect(host.querySelector('.tc-status').textContent).toBe('Читает файл…');
    expect(avatars).toHaveLength(1);
  });

  it('ends with steps, failures and the time since the click', () => {
    card.begin({ key: 'bot:b1', bot });
    card.note({ failed: false });
    card.note({ failed: true });
    vi.advanceTimersByTime(12_000);
    const info = card.end('bot:b1');
    expect(info).toEqual({ ms: 12_000, steps: 2, failed: 1 });
    expect(cards()).toHaveLength(0);
    expect(avatars[0].destroyed).toBe(true);
    vi.advanceTimersByTime(5000);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('turns into an error when the send fails and stops counting', () => {
    card.begin({ key: 'bot:b1', bot });
    vi.advanceTimersByTime(2000);
    card.fail('bot:b1', 'нет связи');
    expect(host.querySelector('.turn-card').dataset.state).toBe('error');
    expect(host.querySelector('.tc-status').textContent).toBe('Не отправилось: нет связи');
    expect(host.querySelector('.tc-stop').hidden).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
    // следующая отправка заменяет карточку с ошибкой и считает с нуля
    card.begin({ key: 'bot:b1', bot });
    expect(cards()).toHaveLength(1);
    expect(host.querySelector('.turn-card').dataset.state).toBe('live');
    expect(host.querySelector('.tc-time').textContent).toBe('0 с');
  });

  it('ignores a failure of another chat', () => {
    card.begin({ key: 'bot:b1', bot });
    card.fail('bot:other', 'x');
    expect(host.querySelector('.turn-card').dataset.state).toBe('live');
  });

  it('Stop before the first event: the click reaches the handler and the card goes away cleanly', () => {
    card.begin({ key: 'bot:b1', bot });
    host.querySelector('.tc-stop').click();
    expect(stops).toBe(1);
    card.end('bot:b1');
    expect(cards()).toHaveLength(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('room: asks who answers first, then shows the chosen bot without a reset', () => {
    card.begin({ key: 'room:r1', phase: 'choosing' });
    expect(host.querySelector('.tc-status').textContent).toBe('Выбираю, кто ответит…');
    expect(host.querySelector('.tc-orb .k-pix')).not.toBeNull();
    vi.advanceTimersByTime(4000);
    card.begin({ key: 'room:r1', bot, name: 'Мира' });
    card.phase('thinking');
    expect(cards()).toHaveLength(1);
    expect(host.querySelector('.tc-name').textContent).toBe('Мира');
    expect(host.querySelector('.tc-orb').textContent).toBe('orb');
    expect(host.querySelector('.tc-time').textContent).toBe('4 с');
    expect(host.querySelector('.tc-status').textContent).toBe('Думает…');
  });

  it('keeps the start time when the chat is redrawn', () => {
    card.begin({ key: 'bot:b1', bot });
    vi.advanceTimersByTime(5000);
    card.detach();
    expect(vi.getTimerCount()).toBe(0);
    host.innerHTML = '';
    vi.advanceTimersByTime(3000);
    card.begin({ key: 'bot:b1', bot });
    expect(host.querySelector('.tc-time').textContent).toBe('8 с');
    // чужой ход кончился, пока чат был закрыт: время забыто
    card.detach();
    host.innerHTML = '';
    card.forget('bot:b1');
    card.begin({ key: 'bot:b1', bot });
    expect(host.querySelector('.tc-time').textContent).toBe('0 с');
  });

  it('stays under the user message that arrives after it', () => {
    card.begin({ key: 'bot:b1', bot });
    const user = document.createElement('div');
    host.append(user);
    card.toEnd();
    expect(host.lastElementChild.classList.contains('turn-card')).toBe(true);
  });

  it('keeps a long or multi-line step text to one short line', () => {
    card.begin({ key: 'bot:b1', bot });
    card.status('Запускает команду' + String.fromCharCode(10) + '$root = ' + 'x'.repeat(400));
    const text = host.querySelector('.tc-status').textContent;
    expect(text).not.toContain(String.fromCharCode(10));
    expect(text.length).toBeLessThanOrEqual(101);
    expect(text.startsWith('Запускает команду $root')).toBe(true);
  });
});
