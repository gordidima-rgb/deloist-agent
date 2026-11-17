<script>
  // Метка-фишка без нажатия (.k-tag): бот, как часто, следующий запуск, прошлый. tone: neutral | accent (оранжевый) | danger (красный, только ошибка).
  let { tone = 'neutral', class: className = '', children, ...rest } = $props();
</script>

<span class="k-tag k-tag--{tone} {className}" {...rest}>{@render children?.()}</span>
