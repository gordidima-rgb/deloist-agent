<script>
  // Выбор бота для шага: выпадающий список кита. Список ботов приносит app.js.
  import { tr } from '../../lib/i18n.js';
  import { Select } from '../kit';

  let { bots = [], value = '', onchange } = $props();

  const options = $derived(bots.map((b) => ({ value: b.id, label: b.name })));
  const missing = $derived(Boolean(value) && !bots.some((b) => b.id === value));
</script>

<div class="wf-bp nodrag nowheel nopan">
  <Select
    {options}
    {value}
    placeholder={missing ? tr('Бот удалён') : bots.length ? tr('Выбрать бота') : tr('Ботов пока нет')}
    onchange={(e) => onchange?.(e.currentTarget.value)}
  />
</div>
