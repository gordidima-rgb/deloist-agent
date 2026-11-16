<script>
  // Плашка «чем занят бот»: вырастает из дока у места бота, прилипнув к нему вогнутыми стыками.
  // Сверху приглушённо прошлый шаг, под ним текущий с девятью пикселями; новые шаги въезжают снизу,
  // ширина подстраивается под текст. Движение конечное: одна транзиция на смену шага.
  import { fly } from 'svelte/transition';

  // now: { key, kind: think|read|act|write|wait, text, meta }; was: { key, text, meta } | null
  let { now, was = null, side = 'right', leaving = false, onresize } = $props();

  const MAX = 300;
  let measure = $state();
  let width = $state(0);
  let grown = false; // не реактивно: только чтобы первый рост шёл из нуля
  const reduce = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const rise = { y: 14, duration: reduce ? 0 : 380, opacity: 0 };
  const sink = { y: -12, duration: reduce ? 0 : 300, opacity: 0 };

  $effect(() => {
    if (!measure) return;
    if (leaving) {
      width = 0;
      return;
    }
    // Текущий шаг 14 px, прошлый 12 px: прошлый занимает примерно 0.86 ширины того же текста.
    const texts = [
      [now?.text, now?.meta, 1, 24],
      [was?.text, was?.meta, 0.86, 0],
    ];
    let widest = 0;
    for (const [text, meta, k, icon] of texts) {
      if (!text) continue;
      measure.textContent = meta ? `${text}  ${meta}` : text;
      widest = Math.max(widest, measure.offsetWidth * k + icon);
    }
    const next = Math.min(MAX, Math.ceil(widest + 32));
    // Первый кадр с нулевой шириной, затем рост: плашка вытекает из дока.
    if (!grown) {
      grown = true;
      requestAnimationFrame(() => (width = next));
    } else width = next;
  });
</script>

<div class="pill" class:left={side === 'left'} class:leaving style:width="{width}px" data-region ontransitionend={(e) => e.propertyName === 'width' && onresize?.()}>
  <div class="pill-lines">
    {#if was}
      {#key was.key}
        <div class="pill-line was" in:fly={rise} out:fly={sink}>
          <span class="pill-text">{was.text}</span>{#if was.meta}<span class="pill-meta">{was.meta}</span>{/if}
        </div>
      {/key}
    {/if}
    {#if now}
      {#key now.key}
        <div class="pill-line now" in:fly={rise} out:fly={sink}>
          <span class="k-pix k-pix--{now.kind || 'act'}" role="img" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
          <span class="pill-text">{now.text}</span>{#if now.meta}<span class="pill-meta">{now.meta}</span>{/if}
        </div>
      {/key}
    {/if}
  </div>
  <span class="pill-measure" bind:this={measure} aria-hidden="true"></span>
</div>
