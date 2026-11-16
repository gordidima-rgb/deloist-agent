<script>
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
    <svg class="s-link__chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="m9 18 6-6-6-6" />
    </svg>
  </span>
</button>
