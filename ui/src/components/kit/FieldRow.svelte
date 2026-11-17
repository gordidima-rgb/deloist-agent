<script module>
  let seq = 0;
</script>

<script>
  // Строка поля формы настроек: серая плитка 28 px и метка 13/600, поле рядом (форма шире 440 px)
  // или под меткой. block — поле на всю ширину под меткой (инструкции), stack — метка всегда сверху
  // (пара выборов в .k-fpair). tip — пояснение в подсказке «i». children получает { id }.
  import Icon from './Icon.svelte';
  import InfoTip from './InfoTip.svelte';

  let { label = '', icon = '', tip = '', tipLabel = '', block = false, stack = false, top = false, for: htmlFor = '', class: className = '', children, ...rest } = $props();

  const uid = `k-frow-${++seq}`;
  const id = $derived(htmlFor || uid);
</script>

<div class="k-frow {className}" class:k-frow--block={block} class:k-frow--stack={stack} class:k-frow--top={top} {...rest}>
  <div class="k-frow__label">
    {#if icon}<span class="k-tile k-tile--sm" aria-hidden="true"><Icon name={icon} size={null} /></span>{/if}
    <label for={id}>{label}</label>
    {#if tip}<InfoTip text={tip} label={tipLabel || label} />{/if}
  </div>
  <div class="k-frow__control">{@render children?.({ id })}</div>
</div>
