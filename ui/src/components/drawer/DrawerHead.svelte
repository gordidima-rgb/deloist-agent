<script>
  // Шапка панели бота: вкладки кита, «Развернуть» и «Свернуть». Содержимое вкладок рисует app.js:
  // он знает данные бота и обработчики, шапка только сообщает, какую вкладку выбрали.
  // Свёрнутая панель (rail) — столбик значков разделов, как свёрнутая левая колонка.
  import Tabs from '../kit/Tabs.svelte';
  import IconButton from '../kit/IconButton.svelte';
  import { tr } from '../../lib/i18n.js';

  let { value = 'profile', full = false, rail = false, onchange, onclose, onexpand, onfull } = $props();

  const items = $derived([
    { id: 'profile', label: tr('Профиль'), icon: ['M20 21a8 8 0 0 0-16 0', 'M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8z'] },
    { id: 'memory', label: tr('Память'), icon: ['M12 5a3 3 0 0 0-5.8-1A3 3 0 0 0 4 9a3 3 0 0 0 0 6 3 3 0 0 0 2.2 5A3 3 0 0 0 12 19z', 'M12 5a3 3 0 0 1 5.8-1A3 3 0 0 1 20 9a3 3 0 0 1 0 6 3 3 0 0 1-2.2 5A3 3 0 0 1 12 19z', 'M12 5v14'] },
    { id: 'files', label: tr('Файлы'), icon: ['M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z', 'M14 3v5h5', 'M9 13h6M9 17h6'] },
    { id: 'routines', label: tr('Рутины'), icon: ['M3 12a9 9 0 0 1 15.5-6.2L21 8', 'M21 3v5h-5', 'M21 12a9 9 0 0 1-15.5 6.2L3 16', 'M3 21v-5h5'] },
    { id: 'goals', label: tr('Цели'), icon: ['M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z', 'M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10z', 'M12 11a1 1 0 1 0 0 2 1 1 0 0 0 0-2z'] },
  ]);

  const fullLabel = $derived(full ? tr('Вернуть рядом с чатом') : tr('Развернуть'));
</script>

{#if rail}
  <div class="drawer-rail-icons" role="toolbar" aria-orientation="vertical" aria-label={tr('Панель бота')}>
    <IconButton label={tr('Раскрыть панель')} id="btn-drawer-expand" onclick={() => onexpand?.()}>
      {#snippet icon()}
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m11 17-5-5 5-5M18 17l-5-5 5-5" /></svg>
      {/snippet}
    </IconButton>
    <span class="drawer-rail-sep" aria-hidden="true"></span>
    {#each items as item (item.id)}
      <IconButton label={item.label} class={item.id === value ? 'is-current' : ''} data-id={item.id} onclick={() => onchange?.(item.id)}>
        {#snippet icon()}
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {#each item.icon as d}<path {d} />{/each}
          </svg>
        {/snippet}
      </IconButton>
    {/each}
  </div>
{:else}
  <div class="drawer-tabs" id="drawer-tabs">
    <Tabs {items} {value} label={tr('Панель бота')} {onchange} />
  </div>
  <div class="drawer-head-actions">
    <IconButton label={fullLabel} id="btn-drawer-full" aria-pressed={full} onclick={() => onfull?.(!full)}>
      {#snippet icon()}
        {#if full}
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" /></svg>
        {:else}
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" /></svg>
        {/if}
      {/snippet}
    </IconButton>
    <IconButton label={tr('Свернуть панель')} id="btn-drawer-close" onclick={() => onclose?.()}>
      {#snippet icon()}
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m13 17 5-5-5-5M6 17l5-5-5-5" /></svg>
      {/snippet}
    </IconButton>
  </div>
{/if}
