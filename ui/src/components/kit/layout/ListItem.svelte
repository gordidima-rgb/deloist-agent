<script>
  // Строка списка ботов, чатов или файлов. Живёт внутри <ul class="k-list">.
  // avatar и context — сниппеты; context — место для кнопки меню «…».
  import Badge from '../status/Badge.svelte';

  import Icon from '../media/Icon.svelte';
  let {
    avatar = null,
    title = '',
    subtitle = '',
    meta = '',
    alignMeta = false,
    badge = null,
    tone = 'brand',
    pinned = false,
    active = false,
    unread = false,
    class: className = '',
    onclick,
    context = null,
    ...rest
  } = $props();
</script>

<li
  class="k-list-item {className}"
  class:is-active={active}
  class:is-unread={unread}
  aria-current={active ? 'true' : undefined}
  {...rest}
>
  {#if avatar}<span class="k-list-item__avatar">{@render avatar()}</span>{/if}
  {#if unread}<span class="k-list-item__unread" aria-hidden="true"></span>{/if}
  {#if onclick}
    <button type="button" class="k-list-item__main" onclick={onclick}>
      {#if pinned}
        <Icon name="pin" size={null} class="k-list-item__pin" />
      {/if}
      <span class="k-list-item__text">
        <span class="k-list-item__title">{title}</span>
        {#if subtitle}<span class="k-list-item__sub">{subtitle}</span>{/if}
      </span>
    </button>
  {:else}
    <div class="k-list-item__main">
      {#if pinned}
        <Icon name="pin" size={null} class="k-list-item__pin" />
      {/if}
      <span class="k-list-item__text">
        <span class="k-list-item__title">{title}</span>
        {#if subtitle}<span class="k-list-item__sub">{subtitle}</span>{/if}
      </span>
    </div>
  {/if}
  {#if meta || alignMeta}<span class="k-list-item__meta">{meta}</span>{/if}
  {#if badge !== null && badge !== ''}
    <span class="k-list-item__badge">
      {#if typeof badge === 'number'}<Badge count={badge} {tone} />{:else}<Badge text={String(badge)} {tone} />{/if}
    </span>
  {/if}
  {#if context}<span class="k-list-item__context">{@render context()}</span>{/if}
</li>
