<script>
  // Значок «i» с пояснением: чёрная подсказка до 240 px по наведению и по фокусу с клавиатуры.
  // Пояснение лежит в подсказке, а не под полем: под полем остаётся только то, что нужно видеть всегда.
  // Перенос строк в text (\n) сохраняется. label — имя кнопки для скринридера.
  // Подсказка не выходит за края формы (.k-sform) и окна: при показе её сдвигает place().
  import Icon from '../media/Icon.svelte';
  import Tooltip from './Tooltip.svelte';

  let { text = '', label = '', placement = 'bottom', class: className = '', ...rest } = $props();

  const MARGIN = 8;
  function place(event) {
    const root = event.currentTarget;
    const tip = root.querySelector('.k-tooltip');
    if (!tip) return;
    const area = (root.closest('.k-sform') ?? document.body).getBoundingClientRect();
    const own = root.getBoundingClientRect();
    const left = Math.max(area.left + MARGIN, 0 + MARGIN);
    const right = Math.min(area.right, window.innerWidth) - MARGIN;
    const width = tip.offsetWidth;
    const wanted = Math.max(left, Math.min(own.left - MARGIN, right - width));
    tip.style.setProperty('--k-tip-left', `${Math.round(wanted - own.left)}px`);
  }
</script>

<Tooltip {text} {placement} wrap align="start" class="k-info {className}" onmouseenter={place} onfocusin={place}>
  <button type="button" class="k-info__btn" aria-label={label} {...rest}>
    <Icon name="info" size={null} />
  </button>
</Tooltip>
