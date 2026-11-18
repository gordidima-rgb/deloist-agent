import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import RoutineSchedule from './RoutineSchedule.svelte';

let host;
let instance;
afterEach(() => {
  if (instance) unmount(instance);
  host?.remove();
  instance = null;
});

async function drain() {
  for (let i = 0; i < 14; i += 1) {
    flushSync();
    await Promise.resolve();
  }
}
async function render(props) {
  host = document.createElement('div');
  document.body.append(host);
  instance = mount(RoutineSchedule, { target: host, props });
  await drain();
}
async function pick(select, value) {
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await drain();
}
const selects = () => [...host.querySelectorAll('select')];

describe('RoutineSchedule', () => {
  it('starts as a daily timed routine and reports the 3rd Saturday of September', async () => {
    const onchange = vi.fn();
    await render({ onchange });
    expect(onchange).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'at', recurrence: { freq: 'daily', interval: 1, weekdays: [], monthday: null } }));
    // у рутины нет варианта «Не повторять»
    expect([...selects()[1].options].map((o) => o.value)).not.toContain('none');

    await pick(selects()[1], 'yearly');
    await pick(selects()[2], 'nth');
    const [, , , nth, day, month] = selects();
    await pick(nth, '3');
    await pick(day, '6');
    await pick(month, '9');
    expect(onchange).toHaveBeenLastCalledWith(
      expect.objectContaining({ recurrence: { freq: 'yearly', interval: 1, weekdays: [6], monthday: null, nth: 3, month: 9 } }),
    );
    expect(host.querySelector('.tr-rec-sum').textContent).toContain('каждую 3-ю субботу сентября');
  });

  it('keeps the start date and switches to a one-time moment and to an interval', async () => {
    const onchange = vi.fn();
    await render({ onchange });
    const start = host.querySelector('input[type="date"]');
    start.value = '2026-12-01';
    start.dispatchEvent(new Event('change', { bubbles: true }));
    await drain();
    expect(onchange).toHaveBeenLastCalledWith(expect.objectContaining({ startOn: '2026-12-01' }));

    await pick(selects()[0], 'once');
    expect(host.querySelector('.tr-rec')).toBeNull();
    expect(onchange).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'once' }));
    await pick(selects()[0], 'every');
    expect(host.querySelector('input[type="time"]')).toBeNull();
  });
});
