<script>
  import { tr } from '../lib/i18n.js';
  import { markdownToText } from '../lib/whatsNew.js';

  let {
    current = '',
    from = '',
    sections = [],
    onclose = null,
  } = $props();

  const date = $derived(sections?.[0]?.date || '');

  const bullets = $derived.by(() => {
    const out = [];
    for (const s of sections || []) {
      const lines = String(s?.markdown || '').split(/\r?\n/);
      for (const line of lines) {
        const m = line.match(/^\s*[-*+]\s+(.*)$/);
        if (m && m[1].trim()) out.push(m[1].trim());
      }
    }
    return out;
  });

  function close() {
    onclose?.();
  }
</script>

<div class="wn-overlay" role="dialog" aria-modal="true" aria-labelledby="wn-title">
  <div class="wn-card">
    <h2 id="wn-title" class="wn-title">{tr('Что нового в Deloist')}</h2>
    <p class="wn-meta">
      {from
        ? tr('Обновлено с {from} до {to} · {date}', { from, to: current, date })
        : tr('Версия {current} · {date}', { current, date })}
    </p>
    <ul class="wn-list">
      {#each bullets as bullet}
        <li class="wn-item">{@html markdownToText(bullet)}</li>
      {/each}
    </ul>
    <div class="wn-actions">
      <button type="button" class="wn-btn" onclick={close}>{tr('Понятно')}</button>
    </div>
  </div>
</div>

<style>
  .wn-overlay {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    padding: var(--k-space-4, 16px);
  }
  .wn-card {
    width: min(560px, 100%);
    max-height: 80vh;
    overflow: auto;
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface, #fff);
    color: var(--k-text, #121212);
    box-shadow: var(--k-shadow-3, 0 28px 70px -12px rgba(0, 0, 0, 0.22));
    padding: var(--k-space-5, 20px);
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
  }
  .wn-title {
    margin: 0;
    font: 800 var(--k-text-xl, 22px) / var(--k-leading-tight, 1.2) var(--k-font-display, system-ui);
  }
  .wn-meta {
    margin: 0;
    color: var(--k-text-3, #6e6e6e);
    font: 400 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .wn-list {
    margin: 0;
    padding-left: var(--k-space-5, 20px);
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .wn-item {
    font: 400 var(--k-text-md, 15px) / 1.5 var(--k-font-body, system-ui);
  }
  .wn-actions {
    display: flex;
    justify-content: flex-end;
  }
  .wn-btn {
    height: 36px;
    padding: 0 var(--k-space-4, 16px);
    border: 0;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-brand, #025bff);
    color: var(--k-on-brand, #fff);
    font: 600 var(--k-text-sm, 13px) / 1 var(--k-font-body, system-ui);
    cursor: pointer;
  }
</style>
