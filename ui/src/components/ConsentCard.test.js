import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ConsentCard from './ConsentCard.svelte';

// jsdom не умеет Element.animate, а переходы окна кита на нём держатся (как в Modal.test.js).
if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    const animation = { onfinish: null, oncancel: null, cancel() {}, finish() {}, addEventListener() {}, removeEventListener() {} };
    queueMicrotask(() => animation.onfinish?.());
    return animation;
  };
}

const docs = [
  { kind: 'pdn', version: '2026-11-13', url: 'https://deloist.ru/legal/pdn' },
  { kind: 'offer', version: '2026-11-13', url: 'https://deloist.ru/legal/offer' },
];

let instance;
let host;

function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(ConsentCard, { target: host, props });
  flushSync();
}

async function drain() {
  for (let i = 0; i < 8; i += 1) {
    flushSync();
    await Promise.resolve();
  }
  flushSync();
}

const accept = () => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Принимаю');
const boxes = () => [...host.querySelectorAll('input[type="checkbox"]')];

function tick(box) {
  box.click();
  flushSync();
}

afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
  host = null;
});

describe('ConsentCard', () => {
  it('starts with every box unchecked and a link per document', () => {
    render({ docs });
    expect(boxes()).toHaveLength(2);
    expect(boxes().every((b) => !b.checked)).toBe(true);
    const links = [...host.querySelectorAll('a')].map((a) => [a.getAttribute('href'), a.getAttribute('target')]);
    expect(links).toEqual([
      ['https://deloist.ru/legal/pdn', '_blank'],
      ['https://deloist.ru/legal/offer', '_blank'],
    ]);
    expect(host.textContent).toContain('DeepSeek');
    expect(host.textContent).toContain('КНР');
    expect(accept().disabled).toBe(true);
  });

  it('enables «Принимаю» only when every document is checked and sends kind and version', async () => {
    const onaccept = vi.fn(async () => {});
    render({ docs, onaccept });
    tick(boxes()[0]);
    expect(accept().disabled).toBe(true);
    tick(boxes()[1]);
    expect(accept().disabled).toBe(false);
    accept().click();
    await drain();
    expect(onaccept).toHaveBeenCalledWith([
      { kind: 'pdn', version: '2026-11-13' },
      { kind: 'offer', version: '2026-11-13' },
    ]);
    expect(host.querySelector('[role="dialog"]')).toBeNull();
  });

  it('keeps the card open with the error when the site refuses', async () => {
    const onaccept = vi.fn(async () => {
      throw new Error('Документы обновились');
    });
    render({ docs, onaccept });
    boxes().forEach(tick);
    accept().click();
    await drain();
    expect(host.querySelector('[role="alert"]').textContent).toContain('Документы обновились');
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it('«Позже» closes without consent', () => {
    const onlater = vi.fn();
    const onaccept = vi.fn();
    render({ docs, onaccept, onlater });
    [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Позже').click();
    flushSync();
    expect(onlater).toHaveBeenCalledTimes(1);
    expect(onaccept).not.toHaveBeenCalled();
  });
});
