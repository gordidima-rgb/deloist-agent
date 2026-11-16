<script>
  // Подсказка «мессенджер не подключён» со ссылкой на настройки.
  import { tr } from '../../lib/i18n.js';
  import WfIcon from './WfIcon.svelte';

  let { name = '', onopen } = $props();
</script>

<div class="wflh nodrag" role="note">
  <WfIcon name="link2" size={14} />
  <span>{tr('{name} не подключён.', { name })}</span>
  <button type="button" onclick={() => onopen?.()}>{tr('Открыть настройки')}</button>
</div>

<style>
  .wflh {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
    padding: 8px 10px;
    border-radius: 12px;
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
    font: 500 12.5px/1.35 var(--k-font-body);
    animation: wflh-in 0.4s var(--k-ease-spring, ease) both;
  }
  .wflh button {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: 700 12.5px/1.35 var(--k-font-body);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  @keyframes wflh-in {
    from {
      opacity: 0;
      transform: translateY(-4px) scale(0.97);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wflh {
      animation: none;
    }
  }
</style>
