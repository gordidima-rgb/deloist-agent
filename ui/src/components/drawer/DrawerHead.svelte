<script>
  // Шапка панели бота: вкладки кита и кнопка «Закрыть». Содержимое вкладок рисует app.js:
  // он знает данные бота и обработчики, шапка только сообщает, какую вкладку выбрали.
  import Tabs from '../kit/Tabs.svelte';
  import IconButton from '../kit/IconButton.svelte';
  import { tr } from '../../lib/i18n.js';

  let { value = 'profile', onchange, onclose } = $props();

  const items = $derived([
    { id: 'profile', label: tr('Профиль') },
    { id: 'memory', label: tr('Память') },
    { id: 'files', label: tr('Файлы') },
    { id: 'goals', label: tr('Цели') },
  ]);
</script>

<div class="drawer-tabs" id="drawer-tabs">
  <Tabs {items} {value} label={tr('Панель бота')} {onchange} />
</div>
<IconButton label={tr('Закрыть')} id="btn-drawer-close" onclick={() => onclose?.()}>
  {#snippet icon()}
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
  {/snippet}
</IconButton>
