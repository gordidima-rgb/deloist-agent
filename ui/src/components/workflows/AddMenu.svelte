<script>
  // Меню «Добавить узел»: поле поиска и группы Входы / Шаги / Поток — это Menu кита с search и группами.
  // allowed — список видов, допустимых для линии, которую бросили на пустом месте (null: любые).
  import { tr } from '../../lib/i18n.js';
  import { KINDS, isEnabled } from '../../lib/workflows/graph.js';
  import { Menu } from '../kit';
  import { kindTitle, kindHint, groupTitle } from './Labels.svelte';
  import WfIcon from './WfIcon.svelte';

  let { allowed = null, x = 0, y = 0, origin = 'bottom', onpick, onclose } = $props();

  const GROUPS = ['inputs', 'steps', 'flow'];

  const items = $derived(
    GROUPS.flatMap((g) => {
      const kinds = Object.keys(KINDS).filter((k) => KINDS[k].group === g && isEnabled(k) && (!allowed || allowed.includes(k)));
      if (!kinds.length) return [];
      return [
        { id: `g-${g}`, kind: 'group', label: groupTitle(g) },
        ...kinds.map((k) => ({ id: k, label: kindTitle(k), sub: kindHint(k), keywords: k })),
      ];
    }),
  );
</script>

<Menu
  class="wf-addmenu nodrag nopan nowheel"
  label={tr('Добавить узел')}
  {items}
  {lead}
  {x}
  {y}
  fromBottom={origin === 'bottom'}
  origin={origin === 'top' ? 'top-left' : 'bottom-left'}
  search={tr('Искать узлы')}
  empty={tr('Ничего не нашлось')}
  onpick={(id) => onpick?.(id)}
  onclose={() => onclose?.()}
/>

{#snippet lead(item)}
  <span class="wf-am__ico"><WfIcon name={item.id} size={17} /></span>
{/snippet}

<style>
  :global(.wf-addmenu) {
    width: 300px;
    max-height: min(520px, 76vh);
    overflow-y: auto;
  }
  :global(.wf-am__ico) {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--k-radius-sm);
    background: var(--k-ink-2);
    color: var(--k-brand);
  }
</style>
