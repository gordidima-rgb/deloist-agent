<script>
  // Значок кита по имени из lib/icons.js: линейный SVG 24x24, цвет берёт у текста рядом (currentColor).
  // size = null — размер задаёт CSS (как у значков с классом i/k-i); stroke 2 — толщина для меню и поиска.
  import { ICONS } from '../../lib/icons.js';

  let { name, size = 18, stroke = 2, class: className = '', label = '', ...rest } = $props();

  const body = $derived(Object.hasOwn(ICONS, name) ? ICONS[name] : '');

  $effect(() => {
    if (!body) console.warn(`unknown icon: ${name}`);
  });
</script>

<svg
  class={className}
  viewBox="0 0 24 24"
  width={size ?? undefined}
  height={size ?? undefined}
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? 'img' : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : 'true'}
  {...rest}
>{@html body}</svg>
