<script>
  // Строка списка ботов, чатов или файлов. Живёт внутри <ul class="k-list">.
  // avatar и context — сниппеты; context — место для кнопки меню «…».
  import Badge from './Badge.svelte';

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
        <svg class="k-list-item__pin" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M6 2.5h4l-.6 3.2 2.1 2.1H4.5l2.1-2.1z" /><path d="M8 7.8V13" />
        </svg>
      {/if}
      <span class="k-list-item__text">
        <span class="k-list-item__title">{title}</span>
        {#if subtitle}<span class="k-list-item__sub">{subtitle}</span>{/if}
      </span>
    </button>
  {:else}
    <div class="k-list-item__main">
      {#if pinned}
        <svg class="k-list-item__pin" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M6 2.5h4l-.6 3.2 2.1 2.1H4.5l2.1-2.1z" /><path d="M8 7.8V13" />
        </svg>
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
