<script>
  // Доска под кнопкой «Нужен ты»: «Нужен ты», «В работе», «Ждёт», «Непрочитанные».
  // Данные готовит app.js (адрес /api/needs и состояние списка слева); здесь только показ.
  // sections: [{ key, title, items: [{ id, where, title, meta, tag }] }]. Пустые разделы не рисуем.
  import { ListRow, Badge, Tag } from '../kit';

  let { sections = [], onpick } = $props();
  const shown = $derived(sections.filter((s) => s.items?.length));
  const whereOf = (item) => (item.meta ? `${item.where} · ${item.meta}` : item.where);
</script>

<div class="nb">
  {#each shown as section (section.key)}
    <div class="nb-section" data-section={section.key}>
      <div class="nb-head">
        <span class="nb-title">{section.title}</span>
        <Badge class="nb-count" count={section.items.length} />
      </div>
      <ul class="k-lrows">
        {#each section.items as item (item.id)}
          <ListRow class="nb-item" title={item.title || whereOf(item)} sub={item.title ? whereOf(item) : ''} onclick={() => onpick?.(section.key, item)}>
            {#snippet trailing()}{#if item.tag}<Tag class="nb-tag" tone="accent">{item.tag}</Tag>{/if}{/snippet}
          </ListRow>
        {/each}
      </ul>
    </div>
  {/each}
</div>

<style>
  .nb { display: flex; flex-direction: column; gap: var(--k-space-3); }
  .nb-section { display: flex; flex-direction: column; gap: var(--k-space-1); }
  .nb-head { display: flex; align-items: center; gap: var(--k-space-2); padding: var(--k-space-1) var(--k-space-3) 0; color: var(--k-text-3); font: 600 var(--k-text-xs) / 1.3 var(--k-font-body); }
</style>
