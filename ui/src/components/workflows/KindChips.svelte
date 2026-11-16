<script>
  // Выбор вида из набора значков: сетка 3 в ряд, выбранный выделен. Служит для видов запуска и мест вывода.
  import { tr } from '../../lib/i18n.js';
  import WfIcon from './WfIcon.svelte';

  /** options [{id, icon}]; label(id) → текст; unavailable(id) → не подключено (значок с точкой). */
  let { options = [], value = '', label = (id) => id, unavailable = () => false, aria = '', onchange } = $props();
</script>

<div class="wfk nodrag" role="radiogroup" aria-label={aria}>
  {#each options as o (o.id)}
    <button type="button" role="radio" class="wfk-chip" class:on={value === o.id} class:is-off={unavailable(o.id)} aria-checked={value === o.id} title={unavailable(o.id) ? tr('Не подключено') : ''} onclick={() => onchange?.(o.id)}>
      <WfIcon name={o.icon} size={15} />
      <span>{label(o.id)}</span>
      {#if unavailable(o.id)}<i class="wfk-dot" aria-hidden="true"></i>{/if}
    </button>
  {/each}
</div>

<style>
  .wfk {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 4px;
    padding: 4px;
    border-radius: 14px;
    background: var(--k-surface-2);
  }
  .wfk-chip {
    position: relative;
    display: grid;
    justify-items: center;
    gap: 3px;
    padding: 7px 2px 6px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--k-text-2);
    font: 600 11.5px/1.1 var(--k-font-body);
    cursor: pointer;
    transition:
      background 150ms var(--k-ease-out, ease),
      color 150ms var(--k-ease-out, ease),
      transform 300ms var(--k-ease-spring, ease),
      box-shadow 200ms var(--k-ease-out, ease);
  }
  .wfk-chip:hover {
    background: var(--k-surface-3);
    color: var(--k-text);
  }
  .wfk-chip:active {
    transform: scale(0.94);
  }
  .wfk-chip.on {
    background: var(--k-surface);
    color: var(--k-text);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.12),
      0 0 0 1px var(--k-line);
  }
  .wfk-chip.is-off:not(.on) {
    opacity: 0.7;
  }
  .wfk-dot {
    position: absolute;
    top: 5px;
    right: 7px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--k-attention);
  }
  @media (prefers-reduced-motion: reduce) {
    .wfk-chip {
      transition: none;
    }
  }
</style>
