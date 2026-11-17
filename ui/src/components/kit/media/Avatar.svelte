<script>
  // Аватар: картинка, эмодзи или инициалы. face — сниппет для живого лица бота
  // (его монтирует старый код через web/avatar.js); ring — цветной обод.
  let {
    src = '',
    emoji = '',
    initials = '',
    name = '',
    label = '',
    size = 40,
    ring = false,
    ringColor = '',
    face = null,
    class: className = '',
    ...rest
  } = $props();

  const shown = $derived(
    initials ||
      (name
        ? name
            .trim()
            .split(/\s+/)
            .slice(0, 2)
            .map((part) => part.charAt(0))
            .join('')
            .toUpperCase()
        : '?'),
  );
  const aria = $derived(label || name || undefined);
  const ringOn = $derived(Boolean(ring));
  const tint = $derived(typeof ring === 'string' ? ring : ringColor || 'var(--k-bot-sky)');
</script>

<span
  class="k-avatar {className}"
  class:k-avatar--ring={ringOn}
  style:--avatar-size="{size}px"
  style:--ring-color={tint}
  role={aria ? 'img' : undefined}
  aria-label={aria}
  {...rest}
>
  {#if face}
    {@render face()}
  {:else if src}
    <img {src} alt="" draggable="false" />
  {:else if emoji}
    <span class="k-avatar__emoji" aria-hidden="true">{emoji}</span>
  {:else}
    <span class="k-avatar__initials" aria-hidden="true">{shown}</span>
  {/if}
</span>
