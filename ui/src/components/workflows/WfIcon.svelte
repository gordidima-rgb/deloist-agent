<script>
  // Значки рабочих процессов: контурные, 24x24, толщина 2 — как остальные значки боковой панели.
  // Сами рисунки лежат в lib/icons.js под именами wf-<имя>; здесь только перевод имени (узлы, кнопки холста).
import { Icon } from '../kit';

  let { name, size = 16 } = $props();

  // Рисунки, которые совпадают с общими значками, берутся оттуда.
  const SHARED = { plus: 'plus', graph: 'workflow', check: 'check', x: 'close', link2: 'link', file: 'file', front: 'wf-dup' };

  const key = $derived(SHARED[name] ?? `wf-${String(name).replaceAll('_', '-')}`);
  const dot = $derived(name === 'more');
</script>

<Icon name={key} {size} stroke={dot ? 3 : 2} class="wf-ico" />
