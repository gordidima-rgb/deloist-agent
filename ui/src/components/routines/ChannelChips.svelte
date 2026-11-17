<script>
  // Куда прислать напоминание: фишки Windows и Telegram. Telegram доступен, только когда чат привязан;
  // без привязки рядом ссылка на настройки. Общие для вкладок «Напоминания» и «Привычки».
  import { tr } from '../../lib/i18n.js';
import { Chip, Button } from '../kit';

  let { windows = $bindable(true), telegram = $bindable(false), paired = false, onopentelegram } = $props();
</script>

<div class="ch">
  <div class="ch-chips">
    <Chip bind:selected={windows}>{tr('Windows')}</Chip>
    <Chip bind:selected={telegram} disabled={!paired} class={paired ? '' : 'ch-off'}>{tr('Telegram')}</Chip>
  </div>
  {#if !paired}
    <p class="ch-hint">
      {tr('Telegram не подключён.')}
      <Button class="ch-link" variant="ghost" size="sm" onclick={() => onopentelegram?.()}>{tr('Открыть настройки Telegram')}</Button>
    </p>
  {/if}
</div>

<style>
  .ch {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .ch-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
  :global(.ch-off) {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ch-hint {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
</style>
