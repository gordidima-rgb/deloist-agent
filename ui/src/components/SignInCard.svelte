<script>
  import { Button } from './kit';
  import { tr } from '../lib/i18n.js';

  let {
    onOpenBrowser = null,
    onDone = null,
    onLater = null,
  } = $props();

  let busy = $state(false);

  async function openBrowser() {
    if (busy) return;
    busy = true;
    try {
      await onOpenBrowser?.();
    } finally {
      busy = false;
    }
  }
</script>

<div class="sic-card">
  <h3 class="sic-title">{tr('Войди в Google в браузере бота')}</h3>
  <p class="sic-text">{tr('Один раз: бот сможет сам подключать бесплатные модели и входить на сайты через Google. Пароль вводишь только ты — бот его не видит.')}</p>
  <div class="sic-actions">
    <Button variant="primary" disabled={busy} onclick={openBrowser}>
      {tr('Открыть браузер')}
    </Button>
    <Button variant="secondary" onclick={() => onDone?.()}>{tr('Готово')}</Button>
    <Button variant="ghost" onclick={() => onLater?.()}>{tr('Позже')}</Button>
  </div>
</div>

<style>
  .sic-card {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2);
    color: var(--k-text);
  }
  .sic-title {
    margin: 0;
    font: 800 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, system-ui);
  }
  .sic-text {
    margin: 0;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.5 var(--k-font-body, system-ui);
  }
  .sic-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
</style>