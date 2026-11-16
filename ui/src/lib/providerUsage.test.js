import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { icon } from './icons.js';

const source = readFileSync('../web/providers.js', 'utf8');
const helpers = source.slice(source.indexOf('  function windows('), source.indexOf('  async function renderUsage('));
const details = new Function('esc', 'tr', 'iconColor', 'limitLabel', 'DeloistUI', `${helpers}\nreturn details;`)(
  (value) => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;'),
  (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => values[key] ?? ''),
  () => 'ffffff', (label) => label, { icon },
);
const provider = { id: 'antigravity', name: 'Gemini', limits: [] };

describe('provider quota explanation', () => {
  it('distinguishes expired authorization from an unsupported quota source', () => {
    const expired = details({ ...provider, limitsStatus: 'auth_expired' });
    expect(expired).toContain('токен недействителен или истёк');
    expect(expired).not.toContain('Этот провайдер не сообщает лимиты');
    expect(details({ ...provider, limitsStatus: 'unsupported' })).toContain('Для входа по API-ключу');
    expect(details({ ...provider, limitsStatus: 'unavailable' })).toContain('Повторим позже');
  });

  it('keeps the last quota visible with its date and stale-data explanation', () => {
    const limits = [{ label: '5h limit', left: 42, resets: '12:00' }];
    const stale = details({ ...provider, limits, limitsStatus: 'auth_expired', at: 1_800_000_000_000 });
    expect(stale).toContain('осталось 42 %');
    expect(stale).toContain('Данные от');
    expect(stale).toContain('токен недействителен или истёк');
    const fresh = details({ ...provider, limits, limitsStatus: 'ok' });
    expect(fresh).toContain('осталось 42 %');
    expect(fresh).not.toContain('токен недействителен');
  });
});
