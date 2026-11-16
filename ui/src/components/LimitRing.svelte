<script>
  // Кольцо лимита бота: дуга по доле value (0–1) и подпись-процент под кольцом.
  // Анимации нет, кроме одной короткой CSS-транзиции дуги при смене значения; при
  // prefers-reduced-motion и она выключена. Бесконечных анимаций нет вообще.
  import { tr } from '../lib/i18n.js';

  let { value = 0, color = 'var(--accent, #6366f1)', size = 56, label = '' } = $props();

  const stroke = $derived(Math.max(3, Math.round(size / 9)));
  const radius = $derived((size - stroke) / 2);
  const circumference = $derived(2 * Math.PI * radius);
  const clamped = $derived(Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0);
  const percent = $derived(Math.round(clamped * 100));
  const offset = $derived(circumference * (1 - clamped));
  const title = $derived(label ? tr('{name}: использовано {percent}%', { name: label, percent }) : tr('Использовано {percent}%', { percent }));
</script>

<div class="limit-ring" style:width="{size}px" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent} aria-label={title} {title}>
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <circle class="track" cx={size / 2} cy={size / 2} r={radius} fill="none" stroke-width={stroke} />
    <circle
      class="arc"
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke={color}
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={circumference}
      stroke-dashoffset={offset}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  <span class="pct">{percent}%</span>
  {#if label}<span class="name">{label}</span>{/if}
</div>

<style>
  .limit-ring {
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    color: var(--text, inherit);
    font-variant-numeric: tabular-nums;
  }
  .track {
    stroke: var(--ring-track, rgba(128, 128, 128, 0.25));
  }
  .arc {
    transition: stroke-dashoffset 240ms ease-out;
  }
  .pct {
    font-size: 12px;
    line-height: 1.1;
    font-weight: 600;
  }
  .name {
    font-size: 11px;
    line-height: 1.1;
    opacity: 0.7;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .arc {
      transition: none;
    }
  }
</style>
