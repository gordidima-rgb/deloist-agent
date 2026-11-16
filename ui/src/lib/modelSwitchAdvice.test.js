// После смены модели: когда предлагать сжать историю. Код живёт в web/app.js (старый DOM-экран),
// тест вырезает правило и запускает его отдельно.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const source = readFileSync(resolve(process.cwd(), '../web/app.js'), 'utf8');
const block = source.slice(source.indexOf('const SWITCH_OFFER_SHARE'), source.indexOf('function toastAction('));
const advice = new Function(`${block}\nreturn modelSwitchAdvice;`)();

describe('model switch advice', () => {
  it('stays quiet for a small conversation', () => {
    expect(advice({ tokens: 20_000, window: 200_000, windowKnown: true, percent: 15 }, false).kind).toBe('none');
  });

  it('offers to compact when history takes 30% of the new window', () => {
    expect(advice({ tokens: 60_000, window: 200_000, windowKnown: true, percent: 40 }, false)).toEqual({ kind: 'offer', tokens: 60_000, share: 30 });
  });

  it('says the program compacts by itself when history is over the threshold', () => {
    expect(advice({ tokens: 300_000, window: 128_000, windowKnown: true, percent: 100 }, false).kind).toBe('auto');
  });

  it('does not offer without a known window or for ACP agents', () => {
    expect(advice({ tokens: 90_000, window: 128_000, windowKnown: false, percent: 50 }, false).kind).toBe('none');
    expect(advice({ tokens: 90_000, window: 128_000, windowKnown: true, percent: 100 }, true).kind).toBe('none');
  });
});
