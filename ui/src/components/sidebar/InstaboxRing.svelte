<script>
  // Кружок Instabox в ряду лимитов: до подключения пунктир, после — кольцо со значком и точкой состояния.
  import { onMount } from 'svelte';
  import { tr } from '../../lib/i18n.js';
  import { instaboxFeed, ringState, clock } from '../../lib/instabox.js';

  let { api, onopen } = $props();

  let snap = $state({ loaded: false, status: null });
  onMount(() => instaboxFeed(api).subscribe((s) => { snap = s; }));

  const view = $derived(ringState(snap.status));
  const text = $derived.by(() => {
    switch (view.kind) {
      case 'running': return tr('работает');
      case 'sleeping': return tr('спит — проснётся от сообщения');
      case 'cooldown': return view.until ? tr('пауза до {time}', { time: clock(view.until) }) : tr('пауза');
      case 'busy': return tr('настраивается');
      case 'offline': return tr('нет связи');
      default: return tr('Подключить бесплатный сервер');
    }
  });
  const label = $derived(view.connected ? `${tr('Бесплатный сервер')}: ${text}` : text);
</script>

{#if snap.loaded}
  <button
    type="button"
    class="usage-item ibx-ring"
    class:ibx-off={!view.connected}
    data-ibx={view.kind}
    title={label}
    aria-label={label}
    onclick={() => onopen?.()}
  >
    <span class="ibx-circle" aria-hidden="true"></span>
    <img src="/icons/apps/instabox.svg" alt="" />
    {#if view.connected}<span class="ibx-dot" aria-hidden="true"></span>{/if}
  </button>
{/if}
