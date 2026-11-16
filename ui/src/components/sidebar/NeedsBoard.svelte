<script>
  // Доска под кнопкой «Нужен ты»: «Нужен ты», «В работе», «Ждёт», «Непрочитанные».
  // Данные готовит app.js (адрес /api/needs и состояние списка слева); здесь только показ.
  // sections: [{ key, title, items: [{ id, where, title, meta, tag }] }]. Пустые разделы не рисуем.
  let { sections = [], onpick } = $props();
  const shown = $derived(sections.filter((s) => s.items?.length));
</script>

<div class="nb">
  {#each shown as section (section.key)}
    <div class="nb-section" data-section={section.key}>
      <div class="nb-head">
        <span class="nb-title">{section.title}</span>
        <span class="nb-count">{section.items.length}</span>
      </div>
      {#each section.items as item (item.id)}
        <button type="button" class="nb-item" onclick={() => onpick?.(section.key, item)}>
          <span class="nb-where">{item.where}{#if item.meta}<span class="nb-meta"> · {item.meta}</span>{/if}</span>
          {#if item.title}<span class="nb-text">{item.title}</span>{/if}
          {#if item.tag}<span class="nb-tag">{item.tag}</span>{/if}
        </button>
      {/each}
    </div>
  {/each}
</div>

<style>
  .nb { display: flex; flex-direction: column; gap: var(--space-2, 8px); }
  .nb-section { display: flex; flex-direction: column; gap: 2px; }
  .nb-head { display: flex; align-items: center; gap: var(--space-2, 8px); padding: var(--space-1, 4px) var(--space-3, 12px) 0; color: var(--color-text-muted); font-size: var(--text-xs, 12px); font-weight: 600; }
  .nb-count { min-width: 18px; padding: 0 5px; border-radius: 9px; background: var(--color-bg-surface-soft); text-align: center; font-weight: 500; }
  .nb-item { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: var(--space-2, 8px) var(--space-3, 12px); border: none; border-radius: var(--radius-md, 10px); background: var(--color-bg-surface-soft); color: var(--color-text-primary); text-align: left; cursor: pointer; font: inherit; }
  .nb-item:hover { background: var(--color-brand-tint); }
  .nb-where { color: var(--color-text-muted); font-size: var(--text-xs, 12px); }
  .nb-text { font-size: var(--text-sm, 14px); overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .nb-tag { color: var(--color-accent-text); font-size: var(--text-xs, 12px); font-weight: 600; }
</style>
