import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import { mountComponent } from '../../lib/mount.svelte.js';
import Stage from './Stage.svelte';
import { lookForPhase, phaseOf } from './model.js';

let instance;
let host;
let api;
let feel;
let set;
let destroyed;

const orbFactory = () => (el) => {
  feel = vi.fn();
  set = vi.fn();
  destroyed = vi.fn();
  return { feel, set, destroy: destroyed };
};

function render(props = {}) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(Stage, {
    target: host,
    props: {
      bot: { id: 'o1', name: 'Оркестратор', skin: 13 },
      stage: 'interview',
      mountOrb: orbFactory(),
      voice: { voiceButton: vi.fn(() => ({ cancel: vi.fn() })) },
      onready: (a) => { api = a; },
      ...props,
    },
  });
  flushSync();
  return host;
}

const bubbles = () => [...host.querySelectorAll('.orch-bub:not(.orch-wait)')].map((b) => ({ user: b.parentElement.classList.contains('is-user'), text: b.textContent.trim() }));
const waitLabel = () => host.querySelector('.orch-wait-label')?.textContent;

beforeEach(() => {
  delete window.CrewAvatarStates;
});

afterEach(() => {
  if (instance?.destroy) instance.destroy();
  else if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
  api = null;
  vi.useRealTimers();
  delete window.CrewAvatarStates;
});

describe('Stage bubbles', () => {
  it('draws history: orchestrator on the left, human on the right', () => {
    render({ history: [{ role: 'assistant', text: 'Какую задачу решаем?' }, { role: 'user', text: 'Писать посты' }, { role: 'tool', text: 'x' }] });
    expect(bubbles()).toEqual([
      { user: false, text: 'Какую задачу решаем?' },
      { user: true, text: 'Писать посты' },
    ]);
  });

  it('shows thinking, then typing dots while text_delta streams, then the final text', () => {
    render();
    api.event({ type: 'turn_start', botId: 'o1' });
    flushSync();
    expect(waitLabel()).toBe('Думает…');
    api.event({ type: 'text_delta', turnId: 't', step: 1, text: 'Привет' });
    api.event({ type: 'text_delta', turnId: 't', step: 1, text: ', давай' });
    flushSync();
    expect(waitLabel()).toBe('Печатает…');
    expect(bubbles()).toEqual([{ user: false, text: 'Привет, давай' }]);
    api.event({ type: 'message', message: { role: 'assistant', text: 'Привет, давай начнём' } });
    api.event({ type: 'turn_end' });
    flushSync();
    expect(bubbles()).toEqual([{ user: false, text: 'Привет, давай начнём' }]);
    expect(host.querySelector('.orch-wait')).toBeNull();
  });

  it('sending adds a right bubble at once, shows thinking, and the server echo is not doubled', async () => {
    const onsend = vi.fn(() => Promise.resolve());
    render({ onsend });
    api.send('  Нужен бот для почты ');
    flushSync();
    expect(onsend).toHaveBeenCalledWith('Нужен бот для почты', {});
    expect(bubbles()).toEqual([{ user: true, text: 'Нужен бот для почты' }]);
    expect(waitLabel()).toBe('Думает…');
    api.event({ type: 'message', message: { role: 'user', text: 'Нужен бот для почты' } });
    flushSync();
    expect(bubbles()).toHaveLength(1);
  });

  it('Enter in the field sends, Shift+Enter does not', () => {
    const onsend = vi.fn(() => Promise.resolve());
    render({ onsend });
    const field = host.querySelector('.orch-input');
    field.value = 'привет';
    field.dispatchEvent(new Event('input', { bubbles: true }));
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true }));
    expect(onsend).not.toHaveBeenCalled();
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(onsend).toHaveBeenCalledWith('привет', {});
  });
});

describe('Stage cards and «Хватит, предлагай»', () => {
  function questionNode() {
    const card = document.createElement('div');
    card.className = 'card question';
    card.innerHTML = '<button data-choice="A">A</button><button data-dismiss>Пропустить</button>';
    return card;
  }

  it('hosts the card from app.js in the stream and drops it when app.js removes it', async () => {
    render();
    const card = questionNode();
    api.addCard(card);
    flushSync();
    expect(host.querySelector('.orch-card .card.question')).toBe(card);
    card.remove();
    await Promise.resolve();
    flushSync();
    expect(host.querySelector('.orch-card')).toBeNull();
  });

  it('a click on an option shows the answer as the human bubble', () => {
    render();
    api.addCard(questionNode());
    flushSync();
    host.querySelector('[data-choice]').click();
    flushSync();
    expect(bubbles()).toEqual([{ user: true, text: 'A' }]);
  });

  it('the button skips the open question and sends the text as a message', () => {
    const onsend = vi.fn(() => Promise.resolve());
    render({ onsend });
    const card = questionNode();
    const dismissed = vi.fn();
    card.querySelector('[data-dismiss]').addEventListener('click', dismissed);
    api.addCard(card);
    flushSync();
    host.querySelector('.orch-cardbox .orch-enough').click();
    flushSync();
    expect(dismissed).toHaveBeenCalledTimes(1);
    expect(onsend).toHaveBeenCalledWith('Хватит, предлагай', {});
  });

  it('without an open question the button sits above the field once the human has spoken', () => {
    const onsend = vi.fn(() => Promise.resolve());
    render({ onsend });
    expect(host.querySelector('.orch-foot .orch-enough')).toBeNull();
    api.send('задача');
    flushSync();
    host.querySelector('.orch-foot .orch-enough').click();
    expect(onsend).toHaveBeenLastCalledWith('Хватит, предлагай', {});
  });
});

describe('Stage phases and mood', () => {
  it('maps the phases', () => {
    expect(phaseOf({ stage: 'interview' })).toBe('listening');
    expect(phaseOf({ stage: 'interview', busy: true })).toBe('thinking');
    expect(phaseOf({ stage: 'interview', busy: true, streaming: true })).toBe('typing');
    expect(phaseOf({ stage: 'interview', busy: true, openCards: 1 })).toBe('asking');
    expect(phaseOf({ stage: 'interview', hearing: true, openCards: 1 })).toBe('listening');
    expect(phaseOf({ stage: 'ready', openCards: 1 })).toBe('done');
  });

  it('takes the look from avatar-states and invents none', () => {
    expect(lookForPhase('thinking', {})).toBeNull();
    expect(lookForPhase('thinking', { stageLook: (p) => ({ thinking: { state: 'work', act: 'search' } })[p] })).toEqual({ state: 'work', act: 'search' });
  });

  it('sets the ready-made look for each phase and a short mood on entry', () => {
    window.CrewAvatarStates = { stageLook: (p) => ({ state: `s-${p}`, act: `a-${p}`, mood: p === 'asking' ? 'surprise' : undefined }) };
    render();
    flushSync();
    expect(set).toHaveBeenLastCalledWith({ state: 's-listening', act: 'a-listening' });
    api.event({ type: 'turn_start' });
    flushSync();
    expect(set).toHaveBeenLastCalledWith({ state: 's-thinking', act: 'a-thinking' });
    api.event({ type: 'text_delta', turnId: 't', step: 1, text: 'а' });
    flushSync();
    expect(set).toHaveBeenLastCalledWith({ state: 's-typing', act: 'a-typing' });
    api.addCard(Object.assign(document.createElement('div'), { className: 'card question' }));
    flushSync();
    expect(set).toHaveBeenLastCalledWith({ state: 's-asking', act: 'a-asking' });
    expect(feel).toHaveBeenLastCalledWith('surprise', 2);
  });

  it('leaves the orb alone when avatar-states has no stageLook', () => {
    render();
    api.addCard(Object.assign(document.createElement('div'), { className: 'card question' }));
    flushSync();
    expect(set).not.toHaveBeenCalled();
    expect(feel).not.toHaveBeenCalled();
  });
});

describe('Stage ready', () => {
  const mountLive = (props) => {
    host = document.createElement('div');
    document.body.append(host);
    const handle = mountComponent(Stage, host, { bot: { id: 'o1', name: 'O', skin: 13 }, stage: 'interview', mountOrb: orbFactory(), voice: {}, ...props });
    flushSync();
    instance = { destroy: handle.destroy };
    return handle;
  };

  it('shrinks the orb to the corner and then calls onleave', () => {
    vi.useFakeTimers();
    const onleave = vi.fn();
    const handle = mountLive({ onleave });
    handle.update({ stage: 'ready' });
    flushSync();
    expect(host.querySelector('.orch').classList.contains('is-leaving')).toBe(true);
    expect(onleave).not.toHaveBeenCalled();
    vi.advanceTimersByTime(600);
    expect(onleave).toHaveBeenCalledTimes(1);
  });

  it('leaves at once with prefers-reduced-motion', async () => {
    const original = window.matchMedia;
    window.matchMedia = () => ({ matches: true });
    try {
      const onleave = vi.fn();
      const handle = mountLive({ onleave });
      handle.update({ stage: 'ready' });
      flushSync();
      await Promise.resolve();
      expect(onleave).toHaveBeenCalledTimes(1);
    } finally {
      window.matchMedia = original;
    }
  });

  it('feels the done mood when the team is ready', () => {
    window.CrewAvatarStates = { stageLook: (p) => (p === 'done' ? { state: 'idle', act: '', mood: 'star' } : null) };
    const handle = mountLive({ onleave: () => {} });
    handle.update({ stage: 'ready' });
    flushSync();
    expect(feel).toHaveBeenLastCalledWith('star', 2);
  });
});

describe('Stage assembly', () => {
  const bots = [
    { id: 'b1', name: 'Редактор', role: 'сжимает новости', persona: 'Ты редактор.\nПишешь коротко.\nБез воды.\nЭту строку не покажем.', skin: 3 },
    { id: 'b2', name: 'Скаут', role: 'ищет источники', persona: 'Ты скаут.', skin: 5 },
  ];
  let joys;
  const mountLive = (props) => {
    host = document.createElement('div');
    document.body.append(host);
    joys = [];
    const handle = mountComponent(Stage, host, {
      bot: { id: 'o1', name: 'O', skin: 13 },
      stage: 'interview',
      mountOrb: orbFactory(),
      mountBotOrb: (el, { bot }) => ({ feel: (m) => joys.push(`${bot.id}:${m}`), destroy: () => {} }),
      voice: {},
      ...props,
    });
    flushSync();
    instance = { destroy: handle.destroy };
    return handle;
  };
  const hiddenIds = () => [...host.querySelectorAll('.orch-bot')].map((b) => b.classList.contains('is-hidden'));
  const texts = () => [...host.querySelectorAll('.orch-bot-text')].map((p) => p.textContent);

  beforeEach(() => vi.useFakeTimers());

  it('reveals new orbs one by one, types the persona (3 lines max), then the rules, then leaves', async () => {
    window.CrewAvatarStates = { stageLook: (p) => ({ state: `s-${p}`, act: '', mood: `m-${p}` }) };
    const onleave = vi.fn();
    const handle = mountLive({ onleave });
    handle.update({ stage: 'ready', newBots: bots, rules: 'Правило 1\nПравило 2' });
    flushSync();
    await vi.advanceTimersByTimeAsync(10);
    flushSync();
    expect(hiddenIds()).toEqual([false, true]);
    expect(set).toHaveBeenLastCalledWith({ state: 's-typing', act: '' });
    await vi.advanceTimersByTimeAsync(1500);
    flushSync();
    expect(texts()[0].length).toBeGreaterThan(0);
    expect(texts()[0].length).toBeLessThan('Ты редактор.\nПишешь коротко.\nБез воды.'.length);
    await vi.advanceTimersByTimeAsync(2500);
    flushSync();
    expect(hiddenIds()).toEqual([false, false]);
    expect(texts()[0]).toBe('Ты редактор.\nПишешь коротко.\nБез воды.');
    expect(joys).toEqual(['b1:joy', 'b2:joy']);
    await vi.advanceTimersByTimeAsync(2000);
    flushSync();
    expect(texts().at(-1)).toBe('Правило 1\nПравило 2');
    expect(feel).toHaveBeenLastCalledWith('m-done', 2);
    expect(onleave).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3000);
    expect(onleave).toHaveBeenCalledTimes(1);
  });

  it('«Пропустить» shows the final state at once', async () => {
    const handle = mountLive({ onleave: () => {} });
    handle.update({ stage: 'ready', newBots: bots, rules: 'Правило' });
    flushSync();
    await vi.advanceTimersByTimeAsync(10);
    host.querySelector('.orch-skip').click();
    flushSync();
    expect(hiddenIds()).toEqual([false, false]);
    expect(texts()[1]).toBe('Ты скаут.');
    expect(texts().at(-1)).toBe('Правило');
    await vi.advanceTimersByTimeAsync(0);
    flushSync();
    expect(host.querySelector('.orch-skip')).toBeNull();
  });

  it('reduced motion: final state with no typing, then leaves', async () => {
    const original = window.matchMedia;
    window.matchMedia = () => ({ matches: true });
    try {
      const onleave = vi.fn();
      const handle = mountLive({ onleave });
      handle.update({ stage: 'ready', newBots: bots, rules: 'Правило' });
      flushSync();
      expect(hiddenIds()).toEqual([false, false]);
      expect(texts()[0]).toBe('Ты редактор.\nПишешь коротко.\nБез воды.');
      expect(joys).toEqual([]);
      await vi.advanceTimersByTimeAsync(1600);
      expect(onleave).toHaveBeenCalledTimes(1);
    } finally {
      window.matchMedia = original;
    }
  });

  it('plays only the bots it is given (rebuild passes only the new ones)', async () => {
    const handle = mountLive({ onleave: () => {} });
    handle.update({ stage: 'ready', newBots: [bots[1]], rules: '' });
    flushSync();
    await vi.advanceTimersByTimeAsync(10);
    expect(hiddenIds()).toEqual([false]);
    expect(host.querySelector('.orch-rules')).toBeNull();
  });

  it('no new bots: no assembly, straight to the corner', async () => {
    const onleave = vi.fn();
    const handle = mountLive({ onleave });
    handle.update({ stage: 'ready', newBots: [] });
    flushSync();
    expect(host.querySelector('.orch-assembly')).toBeNull();
    await vi.advanceTimersByTimeAsync(600);
    expect(onleave).toHaveBeenCalledTimes(1);
  });
});

describe('Stage answered question', () => {
  it('collapses the card after an answer', () => {
    render();
    const card = document.createElement('div');
    card.className = 'card question';
    card.innerHTML = '<button data-choice="A">A</button><button data-dismiss>Пропустить</button>';
    api.addCard(card);
    flushSync();
    host.querySelector('[data-choice]').click();
    flushSync();
    expect(host.querySelector('.orch-cardbox').classList.contains('is-answered')).toBe(true);
    expect(host.querySelector('.orch-cardbox .orch-enough')).toBeNull();
  });
});
