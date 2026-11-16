<script>
  import Icon from '../kit/Icon.svelte';
  // Строка-ссылка раздела настроек: подпись, текущее значение текстом и «›».
  // Нажатие открывает подэкран в той же колонке (onclick приходит от окна настроек).
  let { label = '', sublabel = '', value = '', onclick, id = '' } = $props();
</script>

<button type="button" class="k-row s-link" data-link={id || undefined} {onclick}>
  <span class="k-row__text">
    <span class="k-row__label">{label}</span>
    {#if sublabel}<span class="k-row__sub">{sublabel}</span>{/if}
  </span>
  <span class="k-row__control">
    {#if value}<span class="s-link__value">{value}</span>{/if}
    <Icon name="chevron-right" size={null} stroke={2.5} class="s-link__chev" />
  </span>
</button>
