import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { mountComponent } from '../lib/mount.svelte.js';
import NewBotModel from './NewBotModel.svelte';

const plan = { driver: 'deloist', defaultModel: 'deepseek-flash', models: [{ id: 'deepseek-flash', name: 'DeepSeek Flash' }, { id: 'gpt-6.1-terra', name: 'GPT-6.1 Terra' }] };
const providers = [
  { id: 'codex', kind: 'acp', name: 'ChatGPT', installed: '1', status: 'connected', enabled: true, driver: 'acp:codex', models: [{ id: 'gpt-5', name: 'GPT-5' }, { id: 'gpt-5-mini', name: 'GPT-5 mini' }] },
];

let handle;
let host;
function mount(props) {
  host = document.createElement('div');
  document.body.append(host);
  handle = mountComponent(NewBotModel, host, { providers, plan, settings: {}, value: { driver: '', model: '' }, ...props });
  flushSync();
}
afterEach(() => {
  handle?.destroy();
  host?.remove();
});

const chips = () => [...host.querySelectorAll('.nbm-chip')].map((b) => b.textContent.replace(/\s+/g, ' ').trim());
const checked = () => [...host.querySelectorAll('.nbm-chip[aria-checked="true"]')].map((b) => b.textContent.trim());

describe('NewBotModel', () => {
  it('shows the default and the models right away, no click needed', () => {
    mount();
    expect(chips().slice(0, 4)).toEqual(['как в настройках', 'DeepSeek Flash', 'GPT-6.1 Terra', 'GPT-5']);
    expect(checked()).toEqual(['как в настройках']);
  });

  it('picks a model with one click and reports the driver/model pair', () => {
    const onselect = vi.fn();
    mount({ onselect });
    [...host.querySelectorAll('.nbm-chip')].find((b) => b.textContent.includes('GPT-5')).click();
    flushSync();
    expect(onselect).toHaveBeenCalledWith({ driver: 'acp:codex', model: 'gpt-5' });
  });

  it('picking the default sends an empty pair', () => {
    const onselect = vi.fn();
    mount({ onselect, value: { driver: 'deloist', model: 'deepseek-flash' } });
    expect(checked()).toEqual(['DeepSeek Flash']);
    host.querySelector('.nbm-chip').click();
    expect(onselect).toHaveBeenCalledWith({ driver: '', model: '' });
  });

  it('opens the full list under «Ещё»', () => {
    mount();
    host.querySelector('.nbm-more').click();
    flushSync();
    expect(document.querySelector('.mp-pop') || host.querySelector('.mp-pop')).toBeTruthy();
  });
});
