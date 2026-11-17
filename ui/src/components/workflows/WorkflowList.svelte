<script>
  // Список процессов слева: название, число шагов, статус последнего запуска; создать, переименовать, копировать, удалить.
  import { tr } from '../../lib/i18n.js';
  import { statusLabel, agoText } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';
  import { Button, IconButton, Switch } from '../kit';

  /** items: [{id,title,steps,updated_at,last_run}]; selected: id; handlers приходят от экрана. */
  let { items = [], selected = '', loading = false, onselect, oncreate, onrename, onduplicate, ondelete, onenabled, onmenu } = $props();

  let editing = $state('');
  let draft = $state('');
  let confirming = $state('');
  let confirmTimer = 0;

  function startEdit(item) {
    editing = item.id;
    draft = item.title;
    queueMicrotask(() => document.getElementById(`wfl-edit-${item.id}`)?.select());
  }

  function finishEdit(item, save) {
    const next = draft.trim();
    editing = '';
    if (save && next && next !== item.title) onrename?.(item.id, next);
  }

  function askDelete(item) {
    if (confirming === item.id) {
      clearTimeout(confirmTimer);
      confirming = '';
      ondelete?.(item.id);
      return;
    }
    confirming = item.id;
    clearTimeout(confirmTimer);
    confirmTimer = setTimeout(() => (confirming = ''), 3200);
  }
</script>

<aside class="wfl" aria-label={tr('Рабочие процессы')}>
  <header class="wfl-head">
    <IconButton class="wfl-menu" name="wf-list" size="sm" label={tr('Открыть боковую панель')} onclick={() => onmenu?.()} />
    <h2>{tr('Рабочие процессы')}</h2>
    <IconButton class="wfl-new" name="plus" variant="primary" size="sm" label={tr('Новый процесс')} onclick={() => oncreate?.()} />
  </header>

  <div class="wfl-scroll">
    {#if loading && !items.length}
      <div class="wfl-skel" aria-hidden="true"><span></span><span></span><span></span></div>
    {:else if !items.length}
      <p class="wfl-empty">{tr('Процессов пока нет. Создайте первый: триггер, шаги и вывод на одном холсте.')}</p>
    {/if}

    <ul class="wfl-list">
      {#each items as item, i (item.id)}
        <li style:--i={i}>
          <div class="wfl-item" class:on={item.id === selected} class:off={item.enabled === false} role="presentation">
            <button type="button" class="wfl-main" aria-current={item.id === selected ? 'true' : undefined} onclick={() => onselect?.(item.id)} ondblclick={() => startEdit(item)}>
              <span class="wfl-ico"><WfIcon name="graph" size={16} /></span>
              <span class="wfl-text">
                {#if editing === item.id}
                  <input
                    id="wfl-edit-{item.id}"
                    class="wfl-edit"
                    type="text"
                    maxlength="120"
                    bind:value={draft}
                    onclick={(e) => e.stopPropagation()}
                    onblur={() => finishEdit(item, true)}
                    onkeydown={(e) => {
                      e.stopPropagation();
                      if (e.key === 'Enter') finishEdit(item, true);
                      if (e.key === 'Escape') finishEdit(item, false);
                    }}
                  />
                {:else}
                  <b>{item.title}</b>
                {/if}
                <small>
                  {tr('Шагов: {n}', { n: item.steps ?? 0 })}
                  {#if item.last_run}
                    <span class="wfl-chip" data-status={item.last_run.status}>
                      {statusLabel(item.last_run.status)}
                      {#if item.last_run.at}<i>{agoText(item.last_run.at)}</i>{/if}
                    </span>
                  {/if}
                </small>
              </span>
            </button>
            <span class="wfl-sw">
              <Switch checked={item.enabled !== false} aria-label={tr("Включён")} title={item.enabled === false ? tr('Выключен: расписание, вебхук и мессенджеры его не запустят') : tr('Включён: расписание, вебхук и мессенджеры его запускают')} onchange={(v) => onenabled?.(item.id, v)} />
            </span>
            <span class="wfl-acts">
              <IconButton name="wf-pencil" size="sm" label={tr('Переименовать')} onclick={() => startEdit(item)} />
              <IconButton name="wf-dup" size="sm" label={tr('Дублировать')} onclick={() => onduplicate?.(item.id)} />
              {#if confirming === item.id}
                <Button variant="danger-solid" size="sm" title={tr('Нажмите ещё раз, чтобы удалить')} aria-label={tr('Удалить')} onclick={() => askDelete(item)}>{tr('Удалить?')}</Button>
              {:else}
                <IconButton name="wf-trash" size="sm" label={tr('Удалить')} onclick={() => askDelete(item)} />
              {/if}
            </span>
          </div>
        </li>
      {/each}
    </ul>
  </div>
</aside>

<style>
  .wfl {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--k-surface);
    border-right: 1px solid var(--k-line);
  }
  .wfl-head {
    display: flex;
    align-items: center;
    gap: var(--k-space-2);
    min-height: 60px;
    padding: var(--k-space-3) var(--k-space-3) var(--k-space-3) var(--k-space-4);
  }
  .wfl-head h2 {
    flex: 1;
    margin: 0;
    font: 700 var(--k-text-lg) / 1.2 var(--k-font-display);
  }
  .wfl-head :global(.wfl-menu) {
    display: none;
  }
  .wfl-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--k-space-3) var(--k-space-4);
  }
  .wfl-empty {
    margin: var(--k-space-2) var(--k-space-2) 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfl-list {
    display: grid;
    gap: var(--k-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .wfl-list li {
    animation: wfl-in var(--k-dur-appear) var(--k-ease-spring) both;
    animation-delay: calc(var(--i, 0) * 40ms);
  }
  .wfl-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--k-space-1);
    padding-right: var(--k-space-2);
    border-radius: var(--k-radius-md);
    box-shadow: inset 0 0 0 1px transparent;
    transition:
      background var(--k-dur-micro) var(--k-ease-out),
      box-shadow var(--k-dur-ui) var(--k-ease-out);
  }
  .wfl-item:hover {
    background: var(--k-surface-2);
  }
  .wfl-item.on {
    background: var(--k-brand-tint);
    box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--k-brand) 40%, transparent);
  }
  .wfl-item.off .wfl-main {
    opacity: 0.55;
  }
  .wfl-main {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--k-space-3);
    min-width: 0;
    padding: var(--k-space-3);
    border: 0;
    border-radius: var(--k-radius-md);
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
    transition: transform var(--k-dur-ui) var(--k-ease-press);
  }
  .wfl-main:active {
    transform: scale(0.98);
  }
  .wfl-main:focus-visible {
    outline: 2px solid var(--k-brand-text);
    outline-offset: -2px;
  }
  .wfl-ico {
    display: grid;
    flex: none;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--k-radius-sm);
    background: var(--k-surface);
    color: var(--k-brand);
    box-shadow: inset 0 0 0 1px var(--k-line);
  }
  .wfl-text {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 3px;
  }
  .wfl-text b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 600 var(--k-text-sm) / 1.25 var(--k-font-body);
  }
  .wfl-text small {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font: 400 var(--k-text-xs) / 1.2 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfl-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 7px;
    border-radius: var(--k-radius-xs);
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font: 600 var(--k-text-micro) / 1.5 var(--k-font-body);
  }
  .wfl-chip i {
    font-style: normal;
    font-weight: 400;
    color: var(--k-text-3);
  }
  .wfl-chip[data-status='failed'] {
    background: var(--k-danger-tint);
    color: var(--k-danger-text);
  }
  .wfl-chip[data-status='running'],
  .wfl-chip[data-status='stopping'],
  .wfl-chip[data-status='approval'] {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wfl-edit {
    width: 100%;
    min-width: 0;
    height: 26px;
    padding: 0 var(--k-space-2);
    border: 1px solid var(--k-brand);
    border-radius: var(--k-radius-xs);
    background: var(--k-surface);
    color: var(--k-text);
    font: 600 var(--k-text-sm) / 1 var(--k-font-body);
    box-shadow: var(--k-focus);
    outline: none;
  }
  .wfl-sw {
    display: flex;
    flex: none;
  }
  .wfl-acts {
    display: none;
    align-items: center;
    gap: var(--k-space-0);
  }
  .wfl-item:hover .wfl-acts,
  .wfl-item:focus-within .wfl-acts {
    display: flex;
  }
  .wfl-skel {
    display: grid;
    gap: var(--k-space-2);
  }
  .wfl-skel span {
    height: 54px;
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
    animation: wfl-pulse 1.4s ease-in-out infinite;
  }
  @keyframes wfl-in {
    from {
      opacity: 0;
      transform: translateY(6px);
      filter: blur(4px);
    }
  }
  @keyframes wfl-pulse {
    50% {
      opacity: 0.5;
    }
  }
  @media (max-width: 820px) {
    .wfl-head :global(.wfl-menu) {
      display: inline-flex;
    }
    .wfl-item.on .wfl-acts {
      display: flex;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfl-list li,
    .wfl-skel span {
      animation: none;
    }
  }
</style>
