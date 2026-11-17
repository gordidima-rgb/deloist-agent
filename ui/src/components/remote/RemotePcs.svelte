<script>
  // Боковая панель: раздел на каждый подключённый другой ПК аккаунта, по образцу раздела «Команды».
  // Заголовок — имя ПК (точка «в сети» или «не в сети»), внутри боты этого ПК. Пока ПК не в сети, раздел серый.
  // Всё, что пришло с другого ПК, — только текст; цвет проходит проверку; id — только через enc().
  import { onMount } from 'svelte';
  import ListItem from '../kit/ListItem.svelte';
  import Icon from '../kit/Icon.svelte';
  import RemoteOrb from './RemoteOrb.svelte';
  import { tr } from '../../lib/i18n.js';
  import { enc } from '../../lib/remote.js';
  import { whenText } from '../../lib/when.js';
  import { plainPreview } from '../../lib/preview.js';

  // current: { pc, bot } открытого сейчас удалённого бота; onopen(pc, bot) открывает чат.
  let { api, current = null, onopen } = $props();

  const CLOSED_KEY = 'deloist:sidebar-remote-closed';
  const readClosed = () => {
    try {
      const list = JSON.parse(localStorage.getItem(CLOSED_KEY) || '[]');
      return Object.fromEntries((Array.isArray(list) ? list : []).map((id) => [String(id), true]));
    } catch {
      return {};
    }
  };

  let pcs = $state([]);
  let states = $state({}); // id ПК -> { name, bots }
  let needs = $state({}); // id ПК -> { id бота -> число карточек }
  let closed = $state(readClosed());

  function setOpen(id, open) {
    closed = { ...closed, [id]: !open };
    try {
      localStorage.setItem(CLOSED_KEY, JSON.stringify(Object.keys(closed).filter((key) => closed[key])));
    } catch {
      /* без хранилища раздел просто не запомнится */
    }
  }

  async function loadState(pc) {
    try {
      const data = await api(`/api/remote/${enc(pc)}/state`);
      states = { ...states, [pc]: { name: String(data?.pc ?? ''), bots: Array.isArray(data?.bots) ? data.bots : [] } };
    } catch {
      /* ПК пропал: остаётся прежний список */
    }
  }

  async function loadNeeds(pc) {
    try {
      const data = await api(`/api/remote/${enc(pc)}/needs`);
      const counts = {};
      for (const item of Array.isArray(data?.items) ? data.items : []) counts[item.botId] = (counts[item.botId] ?? 0) + 1;
      needs = { ...needs, [pc]: counts };
    } catch {
      /* без карточек */
    }
  }

  async function load() {
    let data;
    try {
      data = await api('/api/remote/pcs');
    } catch {
      return;
    }
    pcs = Array.isArray(data?.pcs) ? data.pcs : [];
    for (const pc of pcs) {
      if (pc.online) {
        loadState(pc.id);
        loadNeeds(pc.id);
      }
    }
  }

  const timers = new Map();
  function later(key, fn, delay = 400) {
    clearTimeout(timers.get(key));
    timers.set(key, setTimeout(fn, delay));
  }

  function onEvent(event) {
    const ev = event.detail;
    if (ev?.type === 'devices_changed') later('all', load, 600);
    else if (ev?.type === 'remote_event' && typeof ev.pc === 'string' && pcs.some((p) => p.id === ev.pc)) {
      const kind = ev.event?.type;
      if (kind === 'state_changed' || kind === 'journal') later(`s:${ev.pc}`, () => loadState(ev.pc));
      else if (kind === 'approval' || kind === 'approval_closed' || kind === 'question' || kind === 'question_closed') later(`n:${ev.pc}`, () => loadNeeds(ev.pc));
    }
  }

  onMount(() => {
    load();
    window.addEventListener('deloist:event', onEvent);
    window.addEventListener('deloist:remote-changed', load);
    return () => {
      window.removeEventListener('deloist:event', onEvent);
      window.removeEventListener('deloist:remote-changed', load);
      for (const id of timers.values()) clearTimeout(id);
    };
  });

  const previewOf = (bot) => {
    const p = bot.preview;
    if (typeof p === 'string') return plainPreview(p);
    return p && typeof p === 'object' ? plainPreview(p.text ?? '') : '';
  };
  const botsOf = (pc) => (states[pc.id]?.bots ?? []).slice().sort((a, b) => (b.at ?? 0) - (a.at ?? 0));
  const totalNeeds = (pc) => Object.values(needs[pc.id] ?? {}).reduce((sum, n) => sum + n, 0);
</script>

{#if pcs.length}
  <div class="rp" data-remote-pcs>
    {#each pcs as pc (pc.id)}
      {@const open = !closed[pc.id]}
      <section class="rp-pc" class:is-off={!pc.online} data-pc={pc.id}>
        <button type="button" class="rp-head" aria-expanded={open} onclick={() => setOpen(pc.id, !open)}>
          <Icon name="chevron-right" size={14} class="rp-chev{open ? ' is-open' : ''}" />
          <span class="rp-name">{pc.name || tr('ПК')}</span>
          {#if totalNeeds(pc)}<span class="rp-need" title={tr('Нужен ты')}>{totalNeeds(pc)}</span>{/if}
          <span class="rp-state"><span class="rp-dot" class:is-on={pc.online} aria-hidden="true"></span>{pc.online ? tr('в сети') : tr('не в сети')}</span>
        </button>
        {#if open}
          <ul class="k-list rp-list" role="list">
            {#each botsOf(pc) as bot (bot.id)}
              <ListItem
                class="sb-row rp-row"
                data-id={bot.id}
                title={bot.name || tr('Бот')}
                subtitle={previewOf(bot)}
                meta={bot.at ? whenText(bot.at) : ''}
                alignMeta
                active={current?.pc === pc.id && current?.bot?.id === bot.id}
                badge={needs[pc.id]?.[bot.id] ? needs[pc.id][bot.id] : null}
                onclick={pc.online ? () => onopen?.(pc, bot) : undefined}
              >
                {#snippet avatar()}<RemoteOrb {bot} busy={Boolean(bot.busy)} />{/snippet}
              </ListItem>
            {/each}
            {#if !botsOf(pc).length}
              <li class="rp-empty">{pc.online ? tr('Ботов пока нет') : tr('Откроется, когда ПК выйдет в сеть')}</li>
            {/if}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
{/if}

<style>
  .rp { display: flex; flex-direction: column; gap: 2px; padding: 2px 8px 8px; border-top: 1px solid var(--k-line); }
  .rp-head {
    display: flex; align-items: center; gap: 6px; width: 100%; min-height: 32px; padding: 0 var(--k-space-2);
    border: 0; border-radius: var(--k-radius-sm); background: transparent; color: var(--k-text-2); cursor: pointer;
    font: 600 var(--k-text-xs, 12px) / 1.2 var(--k-font-body, inherit); text-align: left;
    transition: background 0.15s cubic-bezier(0.22, 1, 0.36, 1), color 0.15s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .rp-head:hover { background: var(--arc-hover); color: var(--k-text); }
  :global(.rp-chev) { flex: none; transition: transform 0.15s cubic-bezier(0.22, 1, 0.36, 1); }
  :global(.rp-chev.is-open) { transform: rotate(90deg); }
  .rp-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rp-state { display: inline-flex; align-items: center; gap: var(--k-space-1); font-weight: 600; color: var(--k-text-3); }
  .rp-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--k-text-3); }
  .rp-dot.is-on { background: var(--k-brand); }
  .rp-need { min-width: 18px; padding: 0 var(--k-space-1); border-radius: var(--k-radius-pill); background: var(--k-attention); color: var(--k-on-brand); font-size: 11px; line-height: 18px; text-align: center; }
  .rp-pc.is-off .rp-name, .rp-pc.is-off :global(.k-list-item__main) { opacity: 0.55; }
  .rp-pc.is-off :global(.k-list-item__main) { cursor: default; }
  .rp-list { margin: 0; padding: 0; list-style: none; }
  .rp-empty { padding: 6px 12px; font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, inherit); color: var(--k-text-3); list-style: none; }
  @media (prefers-reduced-motion: reduce) { :global(.rp-chev), .rp-head { transition: none; } }
</style>
