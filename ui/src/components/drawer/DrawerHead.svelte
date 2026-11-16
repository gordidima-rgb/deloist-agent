<script>
  // Шапка панели бота: вкладки кита, «Развернуть» и «Свернуть». Содержимое вкладок рисует app.js:
  // он знает данные бота и обработчики, шапка только сообщает, какую вкладку выбрали.
  // Свёрнутая панель (rail) — столбик значков разделов, как свёрнутая левая колонка.
  import Tabs from '../kit/Tabs.svelte';
  import IconButton from '../kit/IconButton.svelte';
  import { tr } from '../../lib/i18n.js';

  import Icon from '../kit/Icon.svelte';
  let { value = 'profile', full = false, rail = false, onchange, onclose, onexpand, onfull } = $props();

  const items = $derived([
    { id: 'profile', label: tr('Профиль'), icon: 'user-round' },
    { id: 'memory', label: tr('Память'), icon: 'brain-halves' },
    { id: 'files', label: tr('Файлы'), icon: 'file-lines' },
    { id: 'routines', label: tr('Рутины'), icon: 'repeat' },
    { id: 'goals', label: tr('Цели'), icon: 'goal' },
  ]);

  const fullLabel = $derived(full ? tr('Вернуть рядом с чатом') : tr('Развернуть'));
</script>

{#if rail}
  <div class="drawer-rail-icons" role="toolbar" aria-orientation="vertical" aria-label={tr('Панель бота')}>
    <IconButton label={tr('Раскрыть панель')} id="btn-drawer-expand" onclick={() => onexpand?.()}>
      {#snippet icon()}
        <Icon name="chevrons-left" size={18} />
      {/snippet}
    </IconButton>
    <span class="drawer-rail-sep" aria-hidden="true"></span>
    {#each items as item (item.id)}
      <IconButton label={item.label} class={item.id === value ? 'is-current' : ''} data-id={item.id} onclick={() => onchange?.(item.id)}>
        {#snippet icon()}
          <Icon name={item.icon} size={18} />
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
          <Icon name="shrink" size={18} />
        {:else}
          <Icon name="expand" size={18} />
        {/if}
      {/snippet}
    </IconButton>
    <IconButton label={tr('Свернуть панель')} id="btn-drawer-close" onclick={() => onclose?.()}>
      {#snippet icon()}
        <Icon name="chevrons-right" size={18} />
      {/snippet}
    </IconButton>
  </div>
{/if}
