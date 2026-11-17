<script>
  // Кольцо лимита «Угли»: лавовая дуга по часовой от 12 часов на тусклой дорожке, внутри лицо (аватар бота
  // в доке, значок провайдера в шапке). Меньше 15% — дуга красная и один раз вспыхивает; пусто — дуги нет,
  // центр серый; предел неизвестен (left = null) — пунктирная дорожка. Выбранное — оранжевое гало.
  // Значки: work — тлеющая точка справа снизу, need — лавовая пилюля («ждёт тебя»), done — точка справа сверху.
  // Число под кольцом не пишем: процент и источник лимита — в label (подсказка и aria-label).
  // Вращения и светофора нет; смена дуги идёт 700 мс (--k-dur-ring).
  let { left = null, size = 52, selected = false, badge = '', count = 0, label = '', face, children, class: className = '', ...rest } = $props();

  const big = $derived(size >= 44);
  const stroke = $derived(big ? 2.5 : 2.2);
  const gap = $derived(big ? 1.5 : 1);
  const radius = $derived((size - stroke) / 2);
  const known = $derived(left !== null && left !== undefined && Number.isFinite(left));
  const value = $derived(known ? Math.min(1, Math.max(0, left)) : 0);
  const empty = $derived(known && value <= 0);
  const low = $derived(known && value > 0 && value < 0.15);
</script>

<span
  class="k-ring {className}"
  class:is-low={low}
  class:is-empty={empty}
  class:is-unknown={!known}
  class:is-sel={selected}
  style:--ring-size="{size}px"
  style:--ring-stroke="{stroke}px"
  style:--k-ring-gap="{gap}px"
  style:--left={value}
  role={label ? 'img' : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : 'true'}
  title={label || undefined}
  {...rest}
>
  <svg viewBox="0 0 {size} {size}" aria-hidden="true">
    <defs>
      <linearGradient id="k-ring-lava" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#ffd25e" />
        <stop offset="0.5" stop-color="#ff9a1f" />
        <stop offset="1" stop-color="#ff5a12" />
      </linearGradient>
    </defs>
    <circle class="k-ring__track" cx={size / 2} cy={size / 2} r={radius} pathLength="100" />
    <circle class="k-ring__arc" cx={size / 2} cy={size / 2} r={radius} pathLength="100" />
  </svg>
  {#if face}{@render face()}{:else}<span class="k-ring__face">{@render children?.()}</span>{/if}
  {#if badge === 'work'}<span class="k-ring__work"></span>{/if}
  {#if badge === 'need'}<span class="k-ring__need">{count > 0 ? count : '!'}</span>{/if}
  {#if badge === 'done'}<span class="k-ring__done"></span>{/if}
</span>
