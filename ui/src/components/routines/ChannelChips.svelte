<script>
  // Куда прислать напоминание: фишки Windows и Telegram. Telegram доступен, только когда чат привязан;
  // без привязки рядом ссылка на настройки. Общие для вкладок «Напоминания» и «Привычки».
  import { tr } from '../../lib/i18n.js';
import { Chip } from '../kit';

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
      <button type="button" class="ch-link" onclick={() => onopentelegram?.()}>{tr('Открыть настройки Telegram')}</button>
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
    color: var(--k-text-3, #8d8f94);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .ch-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--k-brand-text, #0250e0);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .ch-link:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
    border-radius: 4px;
  }
</style>
