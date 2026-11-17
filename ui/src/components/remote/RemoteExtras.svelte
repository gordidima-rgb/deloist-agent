<script>
  // Компактная панель другого ПК: рутины (только чтение и «Запустить») и лимиты. Правятся они на самом ПК.
  import { onMount } from 'svelte';
import { Button } from '../kit';
  import { tr, trServer } from '../../lib/i18n.js';
  import { enc, errorText } from '../../lib/remote.js';
  import { whenText } from '../../lib/when.js';

  let { api, pc } = $props();

  let routines = $state([]);
  let limits = $state(null);
  let error = $state('');
  let notes = $state({});
  let running = $state('');

  onMount(async () => {
    const base = `/api/remote/${enc(pc.id)}`;
    try {
      const data = await api(`${base}/routines`);
      routines = Array.isArray(data?.routines) ? data.routines : [];
    } catch (err) {
      error = trServer(errorText(err));
    }
    try {
      limits = await api(`${base}/limits`);
    } catch (err) {
      error ||= trServer(errorText(err));
    }
  });

  async function run(routine) {
    running = routine.id;
    notes = { ...notes, [routine.id]: '' };
    try {
      await api(`/api/remote/${enc(pc.id)}/routines/${enc(routine.id)}/run`, { method: 'POST' });
      notes = { ...notes, [routine.id]: tr('Запущено') };
    } catch (err) {
      notes = { ...notes, [routine.id]: trServer(errorText(err)) };
    } finally {
      running = '';
    }
  }
</script>

<div class="rx" data-remote-extras>
  {#if error}<p class="rx-error" role="alert">{error}</p>{/if}
  <h4 class="rx-title">{tr('Рутины ПК «{name}»', { name: pc.name })}</h4>
  {#each routines as r (r.id)}
    <div class="rx-row">
      <div class="rx-text">
        <span class="rx-name">{r.name}</span>
        <span class="rx-sub">{r.botName} · {r.schedule}{#if r.nextRun} · {tr('дальше: {when}', { when: whenText(r.nextRun) })}{/if}{#if !r.enabled} · {tr('выключена')}{/if}</span>
        {#if notes[r.id]}<span class="rx-sub">{notes[r.id]}</span>{/if}
      </div>
      <Button size="sm" disabled={!r.enabled || running === r.id} loading={running === r.id} onclick={() => run(r)}>{tr('Запустить')}</Button>
    </div>
  {:else}
    <p class="rx-sub">{tr('Рутин нет')}</p>
  {/each}
  {#if limits}
    <h4 class="rx-title">{tr('Лимиты')}</h4>
    {#each limits.providers ?? [] as provider}
      <div class="rx-row">
        <div class="rx-text">
          <span class="rx-name">{provider.name || provider.id}</span>
          <span class="rx-sub">
            {#each provider.limits ?? [] as l, i}{i ? ' · ' : ''}{l.label ? `${l.label}: ` : ''}{tr('осталось {n}%', { n: l.left ?? '?' })}{#if l.resets}, {l.resets}{/if}{/each}
          </span>
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .rx { display: flex; flex-direction: column; gap: 6px; padding: var(--k-space-3, 12px) var(--k-space-4, 16px); border-bottom: 1px solid var(--k-line); max-height: 260px; overflow-y: auto; animation: rx-in 0.25s cubic-bezier(0.22, 1, 0.36, 1); }
  h4.rx-title { margin: 4px 0 0; font: 600 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, inherit); color: var(--k-text-3); }
  .rx-row { display: flex; align-items: center; gap: var(--k-space-3, 12px); }
  .rx-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .rx-name { font: 600 var(--k-text-sm, 13px) / 1.35 var(--k-font-body, inherit); overflow-wrap: anywhere; }
  .rx-sub { font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, inherit); color: var(--k-text-3); overflow-wrap: anywhere; margin: 0; }
  .rx-error { margin: 0; color: var(--k-attention-text); font: 600 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, inherit); }
  @keyframes rx-in { from { opacity: 0; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .rx { animation: none; } }
</style>
