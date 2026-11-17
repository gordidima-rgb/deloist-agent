<script>
  // «История» (запуски) и «Версии» (сохранённые графы) процесса: выпадающий список под кнопкой верхней панели.
  // mode 'runs': щелчок по запуску показывает его результаты на узлах; 'versions': щелчок открывает просмотр версии.
  import { tr, trServer } from '../../lib/i18n.js';
  import { toMs, stepSeconds } from '../../lib/workflows/graph.js';
  import { statusLabel, sourceLabel, sourceIcon, versionKindLabel, whenText, seconds } from './Labels.svelte';
  import { Button, ListRow } from '../kit';
  import WfIcon from './WfIcon.svelte';

  /** api(path); workflowId; mode; viewedId — какой запуск сейчас показан; onview(run|null); onpreview(version); onclose(). */
  let { api, workflowId, mode = 'runs', viewedId = '', onview, onpreview, onclose } = $props();

  const PAGE = 20;
  let rows = $state.raw([]);
  let before = $state(null);
  let loading = $state(true);
  let failed = $state('');
  let root = $state();

  async function load(more = false) {
    loading = true;
    failed = '';
    try {
      if (mode === 'runs') {
        const q = new URLSearchParams({ limit: String(PAGE) });
        if (more && before) q.set('before', String(before));
        const res = await api(`/api/workflows/${encodeURIComponent(workflowId)}/runs?${q}`);
        rows = more ? [...rows, ...(res.runs || [])] : res.runs || [];
        before = res.next_before ?? null;
      } else {
        const res = await api(`/api/workflows/${encodeURIComponent(workflowId)}/versions`);
        const list = Array.isArray(res) ? res : res.versions || [];
        rows = [...list].sort((a, b) => (b.v ?? 0) - (a.v ?? 0));
        before = null;
      }
    } catch (err) {
      failed = trServer(err?.message || tr('Не получилось'));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    load(false);
  });

  $effect(() => {
    const away = (e) => {
      if (root && !root.contains(e.target) && !e.target.closest?.('[data-wf-history-btn]')) onclose?.();
    };
    const esc = (e) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onclose?.();
      }
    };
    const t = setTimeout(() => window.addEventListener('pointerdown', away, true), 0);
    window.addEventListener('keydown', esc, true);
    return () => {
      clearTimeout(t);
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', esc, true);
    };
  });

  const runSeconds = (r) => stepSeconds({ started_at: r.started_at, ended_at: r.ended_at });
</script>

<div class="wf-hp nodrag nopan nowheel" bind:this={root} role="dialog" aria-label={mode === 'runs' ? tr('История запусков') : tr('Версии процесса')}>
  <header class="wf-hp__head">
    <b>{mode === 'runs' ? tr('История запусков') : tr('Версии процесса')}</b>
    {#if mode === 'runs' && viewedId}
      <Button class="wf-hp__back" variant="secondary" size="sm" onclick={() => onview?.(null)}>{tr('К текущему')}</Button>
    {/if}
  </header>

  <div class="wf-hp__list">
    {#if failed}
      <p class="wf-hp__msg is-err">{failed}</p>
      <Button class="wf-hp__more" variant="secondary" size="sm" onclick={() => load(false)}>{tr('Повторить')}</Button>
    {:else if loading && !rows.length}
      <p class="wf-hp__msg">{tr('Загружаю…')}</p>
    {:else if !rows.length}
      <p class="wf-hp__msg">{mode === 'runs' ? tr('Запусков пока не было.') : tr('Версий пока нет.')}</p>
    {/if}

    {#if mode === 'runs'}
      <ul class="k-lrows wf-hp__rows">
        {#each rows as r (r.id)}
          <ListRow class="wf-hp__row {r.id === viewedId ? 'on' : ''}" title={sourceLabel(r.solo ? 'node' : r.source)} onclick={() => onview?.(r)}>
            {#snippet leading()}<span class="wf-hp__chip" data-status={r.status}>{statusLabel(r.status)}</span>{/snippet}
            {#snippet meta()}<small class="k-lrow__sub">{whenText(toMs(r.started_at))}</small>{/snippet}
            {#snippet trailing()}<span class="wf-hp__sec">{r.ended_at ? seconds(runSeconds(r)) : ''}</span>{/snippet}
          </ListRow>
        {/each}
      </ul>
      {#if before}
        <Button class="wf-hp__more" variant="ghost" size="sm" disabled={loading} onclick={() => load(true)}>{tr('Показать ещё')}</Button>
      {/if}
    {:else}
      <ul class="k-lrows wf-hp__rows">
        {#each rows as v (v.v)}
          <ListRow class="wf-hp__row" icon="wf-versions" title={versionKindLabel(v.kind)} sub={whenText(toMs(v.at))} onclick={() => onpreview?.(v)}>
            {#snippet trailing()}<span class="wf-hp__sec">{tr('Шагов: {n}', { n: v.steps ?? 0 })}</span>{/snippet}
          </ListRow>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .wf-hp {
    position: absolute;
    z-index: 42;
    top: 66px;
    right: 14px;
    width: 340px;
    max-height: min(480px, calc(100% - 90px));
    display: flex;
    flex-direction: column;
    border-radius: 20px;
    border: 1px solid var(--k-line);
    background: var(--k-surface);
    box-shadow: var(--k-shadow-3);
    transform-origin: top right;
    animation: wf-hp-in 0.36s var(--k-ease-spring, ease) both;
  }
  .wf-hp__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px 8px;
  }
  .wf-hp__head b {
    font: 800 15px/1.2 var(--k-font-display);
  }
  .wf-hp__list {
    display: grid;
    gap: 2px;
    padding: 0 8px 10px;
    overflow-y: auto;
  }
  .wf-hp__msg {
    margin: 8px 10px;
    font: 400 13px/1.4 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wf-hp__msg.is-err {
    color: var(--k-attention-text);
  }
  :global(.wf-hp__row.on .k-lrow__main) {
    background: var(--k-surface-2);
  }
  .wf-hp__chip {
    min-width: 66px;
    padding: 3px 9px;
    border-radius: 999px;
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font: 700 11.5px/1.3 var(--k-font-body);
    text-align: center;
  }
  .wf-hp__chip[data-status='done'] {
    background: var(--k-surface-2);
    color: var(--k-brand-text);
  }
  .wf-hp__chip[data-status='failed'],
  .wf-hp__chip[data-status='approval'] {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wf-hp__chip[data-status='running'],
  .wf-hp__chip[data-status='stopping'] {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wf-hp__sec {
    font: 400 12px/1.2 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  @keyframes wf-hp-in {
    from {
      opacity: 0;
      transform: scale(0.92) translateY(-6px);
      filter: blur(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wf-hp {
      animation: none;
    }
  }
</style>
