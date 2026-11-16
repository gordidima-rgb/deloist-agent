<script>
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
    <button type="button" class="sic-btn sic-btn--primary" disabled={busy} onclick={openBrowser}>
      {tr('Открыть браузер')}
    </button>
    <button type="button" class="sic-btn" onclick={() => onDone?.()}>{tr('Готово')}</button>
    <button type="button" class="sic-btn" onclick={() => onLater?.()}>{tr('Позже')}</button>
  </div>
</div>

<style>
  .sic-card {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2, #f0f0f0);
    color: var(--k-text, #121212);
  }
  .sic-title {
    margin: 0;
    font: 800 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, system-ui);
  }
  .sic-text {
    margin: 0;
    color: var(--k-text-2, #4a4a4a);
    font: 400 var(--k-text-sm, 13px) / 1.5 var(--k-font-body, system-ui);
  }
  .sic-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
  .sic-btn {
    height: 34px;
    padding: 0 var(--k-space-4, 16px);
    border: 1px solid var(--k-line-strong, #d2d2d2);
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface, #fff);
    color: var(--k-text, #121212);
    font: 600 var(--k-text-sm, 13px) / 1 var(--k-font-body, system-ui);
    cursor: pointer;
  }
  .sic-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .sic-btn--primary {
    border: 0;
    background: var(--k-brand, #025bff);
    color: var(--k-on-brand, #fff);
  }
</style>