<script>
  // «История» (запуски) и «Версии» (сохранённые графы) процесса: выпадающий список под кнопкой верхней панели.
  // mode 'runs': щелчок по запуску показывает его результаты на узлах; 'versions': щелчок открывает просмотр версии.
  import { tr, trServer } from '../../lib/i18n.js';
  import { toMs, stepSeconds } from '../../lib/workflows/graph.js';
  import { statusLabel, sourceLabel, versionKindLabel, whenText, seconds } from './Labels.svelte';
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
      <button type="button" class="wf-hp__back" onclick={() => onview?.(null)}>{tr('К текущему')}</button>
    {/if}
  </header>

  <div class="wf-hp__list">
    {#if failed}
      <p class="wf-hp__msg is-err">{failed}</p>
      <button type="button" class="wf-hp__more" onclick={() => load(false)}>{tr('Повторить')}</button>
    {:else if loading && !rows.length}
      <p class="wf-hp__msg">{tr('Загружаю…')}</p>
    {:else if !rows.length}
      <p class="wf-hp__msg">{mode === 'runs' ? tr('Запусков пока не было.') : tr('Версий пока нет.')}</p>
    {/if}

    {#if mode === 'runs'}
      {#each rows as r (r.id)}
        <button type="button" class="wf-hp__row" class:on={r.id === viewedId} onclick={() => onview?.(r)}>
          <span class="wf-hp__chip" data-status={r.status}>{statusLabel(r.status)}</span>
          <span class="wf-hp__main">
            <b>{sourceLabel(r.solo ? 'node' : r.source)}</b>
            <small>{whenText(toMs(r.started_at))}</small>
          </span>
          <span class="wf-hp__sec">{r.ended_at ? seconds(runSeconds(r)) : ''}</span>
        </button>
      {/each}
      {#if before}
        <button type="button" class="wf-hp__more" disabled={loading} onclick={() => load(true)}>{tr('Показать ещё')}</button>
      {/if}
    {:else}
      {#each rows as v (v.v)}
        <button type="button" class="wf-hp__row" onclick={() => onpreview?.(v)}>
          <span class="wf-hp__ver"><WfIcon name="versions" size={15} /></span>
          <span class="wf-hp__main">
            <b>{versionKindLabel(v.kind)}</b>
            <small>{whenText(toMs(v.at))}</small>
          </span>
          <span class="wf-hp__sec">{tr('Шагов: {n}', { n: v.steps ?? 0 })}</span>
        </button>
      {/each}
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
  .wf-hp__back {
    height: 28px;
    padding: 0 11px;
    border: 0;
    border-radius: 9px;
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
    font: 700 12px/1 var(--k-font-body);
    cursor: pointer;
    transition: transform 300ms var(--k-ease-spring, ease);
  }
  .wf-hp__back:active {
    transform: scale(0.94);
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
  .wf-hp__row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--k-text);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wf-hp__row:hover,
  .wf-hp__row.on {
    background: var(--k-surface-2);
  }
  .wf-hp__row:active {
    transform: scale(0.98);
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
    background: color-mix(in oklab, var(--k-bot-mint) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-mint) 75%, var(--k-text));
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
  .wf-hp__ver {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 10px;
    background: color-mix(in oklab, var(--k-bot-violet) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-violet) 70%, var(--k-text));
  }
  .wf-hp__main {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .wf-hp__main b {
    font: 600 13.5px/1.2 var(--k-font-body);
  }
  .wf-hp__main small,
  .wf-hp__sec {
    font: 400 12px/1.2 var(--k-font-body);
    color: var(--k-text-3);
    font-variant-numeric: tabular-nums;
  }
  .wf-hp__more {
    margin: 4px 8px 0;
    height: 32px;
    border: 0;
    border-radius: 10px;
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font: 600 12.5px/1 var(--k-font-body);
    cursor: pointer;
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
