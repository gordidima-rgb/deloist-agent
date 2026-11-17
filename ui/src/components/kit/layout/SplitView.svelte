<script>
  // Каркас страницы: боковая колонка и содержимое. Без перетаскивания: ширина задаётся свойством.
  let { side = 'left', width = '300px', sidebar = null, class: className = '', children, ...rest } = $props();

  const track = $derived(typeof width === 'number' ? `${width}px` : width);
</script>

<div class="k-split k-split--{side} {className}" style:--k-split-side={track} {...rest}>
  <aside class="k-split__side">{@render sidebar?.()}</aside>
  <main class="k-split__main">{@render children?.()}</main>
</div>
