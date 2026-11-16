<script>
  // Переключатель «включён / выключен»: тумблер с пружинным ходом, как остальные элементы раздела.
  let { checked = false, label = '', title = '', disabled = false, onchange } = $props();
</script>

<button
  type="button"
  class="wfsw"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {title}
  {disabled}
  onclick={(e) => {
    e.stopPropagation();
    onchange?.(!checked);
  }}
>
  <span class="wfsw-knob"></span>
</button>

<style>
  .wfsw {
    position: relative;
    flex: none;
    width: 34px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 999px;
    background: var(--k-surface-3, #d9dbe0);
    cursor: pointer;
    transition: background 200ms var(--k-ease-out, ease);
  }
  .wfsw.on {
    background: var(--k-bot-mint, #3ecf8e);
  }
  .wfsw-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: translate 320ms var(--k-ease-spring, ease);
  }
  .wfsw.on .wfsw-knob {
    translate: 14px 0;
  }
  .wfsw:active .wfsw-knob {
    scale: 1.15 1;
  }
  .wfsw:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .wfsw:focus-visible {
    outline: 2px solid var(--k-brand);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .wfsw,
    .wfsw-knob {
      transition: none;
    }
  }
</style>
