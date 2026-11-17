<script module>
  import Icon from '../media/Icon.svelte';
  let seq = 0;
</script>

<script>
  // Окно кита: заменяет старые <dialog id="modal-*">. Открывается состоянием open,
  // Esc и щелчок по затемнению закрывают, Tab ходит только внутри, фокус возвращается
  // на то, что открыло окно. variant="sheet" — та же панель снизу.
  import { tick } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { tr } from '../../../lib/i18n.js';

  let {
    open = $bindable(false),
    title = '',
    description = '',
    label = '',
    variant = 'modal',
    size = 'md',
    closeLabel = '',
    class: className = '',
    onclose,
    children,
    footer,
    ...rest
  } = $props();

  const uid = `k-modal-${++seq}`;
  const titleId = `${uid}-title`;
  const descId = `${uid}-desc`;
  const reduce =
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
      : false;
  const inDur = reduce ? 0 : 220;
  const outDur = reduce ? 0 : 160;

  let panel = $state();
  let returnTo = null;
  let wasOpen = false;

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables() {
    if (!panel) return [];
    return [...panel.querySelectorAll(FOCUSABLE)].filter((el) => el.getAttribute('aria-hidden') !== 'true');
  }

  function focusFirst() {
    if (!panel) return;
    const target = panel.querySelector('[autofocus], [data-autofocus]') ?? focusables()[0] ?? panel;
    target.focus?.({ preventScroll: true });
  }

  function restore() {
    if (returnTo && returnTo.isConnected) returnTo.focus?.({ preventScroll: true });
    returnTo = null;
  }

  function close() {
    if (!open) return;
    open = false;
    onclose?.();
  }

  function keydown(event) {
    if (!open) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (event.key !== 'Tab') return;
    const list = focusables();
    if (!list.length) {
      event.preventDefault();
      panel?.focus();
      return;
    }
    const first = list[0];
    const last = list[list.length - 1];
    const inside = panel?.contains(document.activeElement);
    if (event.shiftKey && (!inside || document.activeElement === first)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (!inside || document.activeElement === last)) {
      event.preventDefault();
      first.focus();
    }
  }

  $effect(() => {
    if (open) {
      if (wasOpen) return;
      wasOpen = true;
      returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      tick().then(() => {
        if (open) focusFirst();
      });
    } else if (wasOpen) {
      wasOpen = false;
      restore();
    }
  });

  $effect(() => () => {
    if (wasOpen) restore();
  });
</script>

<svelte:window onkeydown={keydown} />

{#if open}
  <div class="k-modal" class:is-sheet={variant === 'sheet'} role="presentation" in:fade={{ duration: inDur }} out:fade={{ duration: outDur }} onpointerdown={(event) => event.target === event.currentTarget && close()}>
    <div
      class="k-modal__panel {className}"
      class:k-modal__panel--sm={size === 'sm'}
      class:k-modal__panel--lg={size === 'lg'}
      class:k-modal__panel--sheet={variant === 'sheet'}
      role="dialog"
      aria-modal="true"
      aria-labelledby={title ? titleId : undefined}
      aria-describedby={description ? descId : undefined}
      aria-label={!title ? label || tr('Окно') : undefined}
      tabindex="-1"
      bind:this={panel}
      in:fly={{ y: variant === 'sheet' ? 28 : 12, duration: inDur, opacity: 0 }}
      out:fly={{ y: variant === 'sheet' ? 18 : 8, duration: outDur, opacity: 0 }}
      {...rest}
    >
      {#if title || description}
        <header class="k-modal__head">
          <div class="k-modal__text">
            {#if title}<h2 class="k-modal__title" id={titleId}>{title}</h2>{/if}
            {#if description}<p class="k-modal__desc" id={descId}>{description}</p>{/if}
          </div>
          <button type="button" class="k-btn k-btn--ghost k-btn--icon k-modal__close" aria-label={closeLabel || tr('Закрыть')} title={closeLabel || tr('Закрыть')} onclick={close}>
            <Icon name="close" size={null} />
          </button>
        </header>
      {/if}
      <div class="k-modal__body k-sharpen-in">{@render children?.()}</div>
      {#if footer}<footer class="k-modal__foot">{@render footer()}</footer>{/if}
    </div>
  </div>
{/if}
