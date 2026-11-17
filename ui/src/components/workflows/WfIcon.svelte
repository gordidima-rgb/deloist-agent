<script>
  // Значки рабочих процессов: контурные, 24x24, толщина 2 — как остальные значки боковой панели.
  // Сами рисунки лежат в lib/icons.js под именами wf-<имя>; здесь только перевод имени (узлы, кнопки холста).
import { Icon } from '../kit';
  import { wfIconKey } from '../../lib/workflows/iconKey.js';

  let { name, size = 16 } = $props();

  const key = $derived(wfIconKey(name));
  const dot = $derived(name === 'more');
</script>

<Icon name={key} {size} stroke={dot ? 3 : 2} class="wf-ico" />
