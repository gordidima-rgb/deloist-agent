<script>
  // Доска под кнопкой «Нужен ты»: «Нужен ты», «В работе», «Ждёт», «Непрочитанные».
  // Данные готовит app.js (адрес /api/needs и состояние списка слева); здесь только показ.
  // sections: [{ key, title, items: [{ id, where, title, meta, tag, actions: [{ key, label }] }] }]. Пустые разделы не рисуем.
  // action — кнопка раздела в шапке («Скрыть» у «Сделано, пока тебя не было»): нажатие — onsection(key).
  // actions — кнопки справа у строки (прерванный ход: «Продолжить» и «Отменить»); нажатие — onaction(item, key).
  import { ListRow, Badge, Tag, Button } from '../kit';

  let { sections = [], onpick, onaction, onsection } = $props();
  const shown = $derived(sections.filter((s) => s.items?.length));
  const whereOf = (item) => (item.meta ? `${item.where} · ${item.meta}` : item.where);
</script>

<div class="nb">
  {#each shown as section (section.key)}
    <div class="nb-section" data-section={section.key}>
      <div class="nb-head">
        <span class="nb-title">{section.title}</span>
        <Badge class="nb-count" count={section.items.length} />
        {#if section.action}<Button class="nb-section-action" size="sm" variant="ghost" onclick={() => onsection?.(section.key)}>{section.action}</Button>{/if}
      </div>
      <ul class="k-lrows">
        {#each section.items as item (item.id)}
          <ListRow class="nb-item" title={item.title || whereOf(item)} sub={item.title ? whereOf(item) : ''} onclick={() => onpick?.(section.key, item)}>
            {#snippet trailing()}
              {#if item.tag}<Tag class="nb-tag" tone="accent">{item.tag}</Tag>{/if}
              {#each item.actions || [] as action (action.key)}
                <Button size="sm" variant={action.key === 'continue' ? 'primary' : 'ghost'} onclick={() => onaction?.(item, action.key)}>{action.label}</Button>
              {/each}
            {/snippet}
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
  .nb-head :global(.nb-section-action) { margin-left: auto; }
</style>
