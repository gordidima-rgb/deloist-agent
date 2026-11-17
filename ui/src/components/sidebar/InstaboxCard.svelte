<script>
  // Карточка над нижним меню: предлагает бесплатный сервер, пока Instabox не подключён.
  import { onMount } from 'svelte';
  import Button from '../kit/Button.svelte';
  import Icon from '../kit/Icon.svelte';
  import { tr } from '../../lib/i18n.js';
  import { instaboxFeed, cardVisible, readDismissed, writeDismissed } from '../../lib/instabox.js';

  let { api, onopen } = $props();

  let snap = $state({ loaded: false, status: null });
  let dismissed = $state(readDismissed());
  onMount(() => instaboxFeed(api).subscribe((s) => { snap = s; }));

  const visible = $derived(cardVisible({ ...snap, dismissed }));

  function close() {
    dismissed = true;
    writeDismissed();
  }
</script>

{#if visible}
  <section class="ibx-card" aria-label={tr('Бесплатный сервер')}>
    <button type="button" class="ibx-x" aria-label={tr('Скрыть')} title={tr('Скрыть')} onclick={close}>
      <Icon name="close" size={12} />
    </button>
    <div class="ibx-head">
      <img src="/icons/apps/instabox.svg" alt="" />
      <b>{tr('Бесплатный сервер')}</b>
    </div>
    <p>{tr('Боты отвечают в Telegram, даже когда компьютер выключен')}</p>
    <Button class="ibx-btn" onclick={() => onopen?.()}>{tr('Подключить бесплатный сервер')}</Button>
  </section>
{/if}
