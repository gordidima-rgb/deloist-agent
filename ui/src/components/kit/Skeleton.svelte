<script>
  // Заглушка будущего содержимого. Без бесконечного перелива: в покое не двигается.
  let { width = '100%', height = '12px', circle = false, class: className = '', ...rest } = $props();
</script>

<span
  class="k-skeleton {className}"
  class:is-circle={circle}
  style:width={typeof width === 'number' ? `${width}px` : width}
  style:height={typeof height === 'number' ? `${height}px` : height}
  aria-hidden="true"
  {...rest}
></span>
