<script>
  // Список слева в окне «Подключить свою ИИ»: группы со строками кита, логотип, название и статус.
  // Данные готовит web/providers.js (там же тексты статусов); компонент только рисует и сообщает о выборе.
import { ListItem, Button } from '../kit';

  let { groups = [], selected = '', label = '', onselect, onaction } = $props();

  // Значок не скачался (нет сети, нет в реестре): вместо него первая буква.
  function lost(event, letter) {
    event.currentTarget.replaceWith(document.createTextNode(letter));
  }
</script>

{#snippet logo(row)}
  <span class="prov-logo" class:prov-logo--plus={row.plus} aria-hidden="true">
    {#if row.svg}
      {@html row.svg}
    {:else if row.img}
      <img src={row.img} alt="" onerror={(event) => lost(event, row.letter)} />
    {:else}
      {row.letter}
    {/if}
  </span>
{/snippet}

<ul class="k-list prov-ul" aria-label={label || undefined}>
  {#each groups as group (group.key)}
    {#if group.title}
      <li class="prov-group" role="presentation">
        <span>{group.title}</span>
        {#if group.action}
          <Button size="sm" variant="ghost" class="prov-group__action" onclick={() => onaction?.(group.action.name)}>{group.action.label}</Button>
        {/if}
      </li>
    {/if}
    {#if group.note}<li class="prov-note" role="presentation">{group.note}</li>{/if}
    {#each group.rows as row}
      {#snippet mark()}{@render logo(row)}{/snippet}
      <ListItem
        class="prov-item"
        active={row.id === selected}
        title={row.name}
        badge={row.badge}
        tone={row.tone}
        data-id={row.id}
        avatar={mark}
        onclick={() => onselect?.(row.id)}
      />
    {/each}
  {/each}
</ul>

<style>
  .prov-ul { padding: 0 var(--k-space-1); }
  .prov-group {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-2);
    margin: var(--k-space-3) var(--k-space-2) var(--k-space-1);
    color: var(--k-text-3);
    font: 700 var(--k-text-xs) / 1.3 var(--k-font-body);
    letter-spacing: 0.02em;
    white-space: nowrap;
  }
  .prov-group:first-child { margin-top: 0; }
  .prov-group :global(.prov-group__action) { flex: none; }
  .prov-group span { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .prov-note {
    margin: 0 var(--k-space-2) var(--k-space-2);
    color: var(--k-text-3);
    font: 400 var(--k-text-xs) / var(--k-leading-body) var(--k-font-body);
  }
  :global(.prov-item) { min-height: 48px; }
  .prov-logo {
    display: grid;
    place-items: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
    color: var(--k-text);
    font: 700 var(--k-text-sm) / 1 var(--k-font-body);
  }
  .prov-logo :global(svg),
  .prov-logo img { width: 18px; height: 18px; display: block; }
  .prov-logo--plus { color: var(--k-text-2); font-size: var(--k-text-lg); font-weight: 600; }
  /* Панель слева на --k-surface-2: наведение темнее, выбранная строка поднимается на --k-surface. */
  /* Логотип стоит рядом с кнопкой строки, а не внутри: растягиваем её нажимаемую область на всю строку. */
  .prov-ul :global(.k-list-item__main)::after { content: ''; position: absolute; inset: 0; border-radius: inherit; }
  .prov-ul :global(.k-list-item:hover) { background: var(--k-surface-3); }
  .prov-ul :global(.k-list-item.is-active) { background: var(--k-surface); box-shadow: var(--k-shadow-1); }
  .prov-ul :global(.k-badge--neutral) { background: var(--k-surface-3); }
  .prov-ul :global(.k-list-item.is-active .k-badge--neutral) { background: var(--k-surface-2); }
  .prov-ul :global(.k-list-item.is-active .prov-logo) { background: var(--k-surface-2); }
</style>
