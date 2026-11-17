<script>
  // Подсказка «мессенджер не подключён» со ссылкой на настройки.
  import { tr } from '../../lib/i18n.js';
  import WfIcon from './WfIcon.svelte';
  import { Button } from '../kit';

  let { name = '', onopen } = $props();
</script>

<div class="wflh nodrag" role="note">
  <WfIcon name="link2" size={14} />
  <span>{tr('{name} не подключён.', { name })}</span>
  <Button variant="ghost" size="sm" onclick={() => onopen?.()}>{tr('Открыть настройки')}</Button>
</div>

<style>
  .wflh {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-1) var(--k-space-2);
    padding: var(--k-space-2) var(--k-space-3);
    border-radius: var(--k-radius-md);
    background: var(--k-brand-tint);
    color: var(--k-brand-text);
    font: 500 var(--k-text-xs) / 1.35 var(--k-font-body);
    animation: wflh-in var(--k-dur-appear) var(--k-ease-spring) both;
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
