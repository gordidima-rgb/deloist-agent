<script>
  // Список процессов слева: название, число шагов, статус последнего запуска; создать, переименовать, копировать, удалить.
  import { tr } from '../../lib/i18n.js';
  import { statusLabel, agoText } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';
  import WfSwitch from './WfSwitch.svelte';

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
    <button type="button" class="wfl-menu" title={tr('Открыть боковую панель')} aria-label={tr('Открыть боковую панель')} onclick={() => onmenu?.()}>
      <WfIcon name="list" size={18} />
    </button>
    <h2>{tr('Рабочие процессы')}</h2>
    <button type="button" class="wfl-new" title={tr('Новый процесс')} aria-label={tr('Новый процесс')} onclick={() => oncreate?.()}>
      <WfIcon name="plus" size={16} />
    </button>
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
              <WfSwitch checked={item.enabled !== false} label={tr('Включён')} title={item.enabled === false ? tr('Выключен: расписание, вебхук и мессенджеры его не запустят') : tr('Включён: расписание, вебхук и мессенджеры его запускают')} onchange={(v) => onenabled?.(item.id, v)} />
            </span>
            <span class="wfl-acts">
              <button type="button" title={tr('Переименовать')} aria-label={tr('Переименовать')} onclick={() => startEdit(item)}><WfIcon name="pencil" size={14} /></button>
              <button type="button" title={tr('Дублировать')} aria-label={tr('Дублировать')} onclick={() => onduplicate?.(item.id)}><WfIcon name="dup" size={14} /></button>
              <button type="button" class:is-danger={confirming === item.id} title={confirming === item.id ? tr('Нажмите ещё раз, чтобы удалить') : tr('Удалить')} aria-label={tr('Удалить')} onclick={() => askDelete(item)}>
                {#if confirming === item.id}<span class="wfl-sure">{tr('Удалить?')}</span>{:else}<WfIcon name="trash" size={14} />{/if}
              </button>
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
    gap: 8px;
    min-height: 60px;
    padding: 10px 12px 10px 16px;
  }
  .wfl-head h2 {
    flex: 1;
    margin: 0;
    font: 800 17px/1.2 var(--k-font-display);
    letter-spacing: -0.01em;
  }
  .wfl-menu {
    display: none;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
  }
  .wfl-new {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border: 0;
    border-radius: 12px;
    background: var(--k-ink, #000);
    color: var(--k-on-ink, #fff);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfl-new:hover {
    background: var(--k-ink-3, #24262c);
  }
  .wfl-new:active {
    transform: scale(0.92);
  }
  .wfl-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 12px;
  }
  .wfl-empty {
    margin: 8px 10px;
    font: 400 13px/1.5 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfl-list {
    display: grid;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .wfl-list li {
    animation: wfl-in 0.45s var(--k-ease-out, ease) both;
    animation-delay: calc(var(--i, 0) * 35ms);
  }
  .wfl-item {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 14px;
    transition: background 150ms var(--k-ease-out, ease);
  }
  .wfl-item:hover {
    background: var(--k-surface-2);
  }
  .wfl-item.on {
    background: var(--k-surface-2);
    box-shadow: inset 0 0 0 1px var(--k-line-strong);
  }
  .wfl-sw {
    flex: none;
    padding-right: 12px;
  }
  .wfl-item.off .wfl-main {
    opacity: 0.55;
  }
  .wfl-main {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 11px;
    min-width: 0;
    padding: 9px 10px;
    border: 0;
    border-radius: 14px;
    background: transparent;
    color: var(--k-text);
    text-align: left;
    cursor: pointer;
    transition: transform 300ms var(--k-ease-spring, ease);
  }
  .wfl-main:active {
    transform: scale(0.98);
  }
  .wfl-ico {
    display: grid;
    place-items: center;
    flex: none;
    width: 34px;
    height: 34px;
    border-radius: 11px;
    background: color-mix(in oklab, var(--k-bot-violet) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-violet) 70%, var(--k-text));
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
    font: 700 13.5px/1.25 var(--k-font-body);
  }
  .wfl-text small {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font: 400 12px/1.3 var(--k-font-body);
    color: var(--k-text-3);
  }
  .wfl-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--k-surface-3);
    color: var(--k-text-2);
    font: 600 11px/1.5 var(--k-font-body);
  }
  .wfl-chip i {
    font-style: normal;
    font-weight: 400;
    opacity: 0.7;
  }
  .wfl-chip[data-status='done'] {
    background: color-mix(in oklab, var(--k-bot-mint) 22%, var(--k-surface));
    color: color-mix(in oklab, var(--k-bot-mint) 75%, var(--k-text));
  }
  .wfl-chip[data-status='failed'],
  .wfl-chip[data-status='approval'] {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfl-chip[data-status='running'],
  .wfl-chip[data-status='stopping'] {
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
  }
  .wfl-edit {
    width: 100%;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--k-brand);
    border-radius: 7px;
    background: var(--k-surface);
    color: var(--k-text);
    font: 700 13.5px/1 var(--k-font-body);
    box-shadow: var(--k-focus);
    outline: none;
  }
  .wfl-acts {
    position: absolute;
    right: 52px;
    top: 50%;
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 11px;
    background: var(--k-surface-2);
    opacity: 0;
    translate: 6px -50%;
    pointer-events: none;
    transition:
      opacity 150ms var(--k-ease-out, ease),
      translate 220ms var(--k-ease-out, ease);
  }
  .wfl-item:hover .wfl-acts,
  .wfl-item:focus-within .wfl-acts {
    opacity: 1;
    translate: 0 -50%;
    pointer-events: auto;
  }
  .wfl-acts button {
    display: grid;
    place-items: center;
    min-width: 26px;
    height: 26px;
    padding: 0 4px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--k-text-2);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease);
  }
  .wfl-acts button:hover {
    background: var(--k-surface-3);
    color: var(--k-text);
  }
  .wfl-acts button:active {
    transform: scale(0.9);
  }
  .wfl-acts button.is-danger {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .wfl-sure {
    font: 700 11.5px/1 var(--k-font-body);
    white-space: nowrap;
  }
  .wfl-skel {
    display: grid;
    gap: 6px;
    padding: 4px 6px;
  }
  .wfl-skel span {
    height: 52px;
    border-radius: 14px;
    background: var(--k-surface-2);
  }
  @keyframes wfl-in {
    from {
      opacity: 0;
      transform: translateX(-8px);
    }
  }
  @media (max-width: 820px) {
    .wfl-menu {
      display: grid;
    }
    .wfl-item.on .wfl-acts {
      opacity: 1;
      translate: 0 -50%;
      pointer-events: auto;
    }
    .wfl-item.on .wfl-main {
      padding-right: 104px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wfl-list li {
      animation: none;
    }
  }
</style>
