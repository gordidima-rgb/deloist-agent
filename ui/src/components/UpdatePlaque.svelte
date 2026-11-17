<script>
  // Плашка внизу боковой панели: версия уже стоит на месте, осталось перезапустить.
  // «Что нового» открывает заметки этой версии, пока она ещё не запущена.
import { Button } from './kit';
  import { tr } from '../lib/i18n.js';

  let {
    version = '',
    // Перезапуск ждёт, пока боты закончат работу.
    pending = false,
    error = '',
    onrestart = null,
    onwhatsnew = null,
  } = $props();
</script>

<section class="up-plaque" aria-live="polite">
  <p class="up-title">{tr('Deloist {version} готов', { version })}</p>
  {#if pending}
    <p class="up-note">{tr('Перезапущусь, когда боты закончат работу.')}</p>
  {:else if error}
    <p class="up-note up-note--error">{error}</p>
  {/if}
  <div class="up-row">
    <button type="button" class="up-link" onclick={() => onwhatsnew?.()}>{tr('Что нового')}</button>
    <Button variant="primary" size="sm" disabled={pending} onclick={() => onrestart?.()}>{tr('Перезапустить')}</Button>
  </div>
</section>

<style>
  .up-plaque {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    margin: var(--k-space-2, 8px) var(--k-space-1, 4px) 0;
    padding: var(--k-space-3, 12px);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-surface);
    color: var(--k-text);
  }
  .up-title {
    margin: 0;
    font: 600 var(--k-text-sm, 13px) / 1.3 var(--k-font-body, system-ui);
  }
  .up-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-xs, 11px) / 1.4 var(--k-font-body, system-ui);
  }
  .up-note--error {
    color: var(--k-text-2);
  }
  .up-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2, 8px);
  }
  .up-link {
    padding: var(--k-space-1, 4px) 0;
    border: 0;
    background: none;
    color: var(--k-text-2);
    font: 400 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .up-link:hover {
    color: var(--k-text);
  }
</style>
