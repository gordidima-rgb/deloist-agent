<script>
  // Выбор вида из набора значков: сетка-переключатель кита (ChoiceGrid). Служит для видов запуска и мест вывода.
  // Вариант без подключения не прячется, а помечен оранжевой точкой.
  import { tr } from '../../lib/i18n.js';
  import { ChoiceGrid } from '../kit';
  import WfIcon from './WfIcon.svelte';

  /** options [{id, icon}]; label(id) → текст; unavailable(id) → не подключено (значок с точкой). */
  let { options = [], value = '', label = (id) => id, unavailable = () => false, aria = '', onchange } = $props();

  const items = $derived(options.map((o) => ({ id: o.id, icon: o.icon, label: label(o.id), need: unavailable(o.id), title: unavailable(o.id) ? tr('Не подключено') : '' })));
</script>

<ChoiceGrid class="wfk nodrag" {items} {value} label={aria} onchange={(id) => onchange?.(id)}>
  {#snippet icon(item)}<WfIcon name={item.icon} size={16} />{/snippet}
</ChoiceGrid>
