<script>
  // Ползунок на нативном input[type=range], дорожка и бегунок - на токенах кита.
  // Стрелки, Home/End, PageUp/PageDown даёт сам input; valueText читают скринридеры (aria-valuetext).
  // marks - значения, у которых под дорожкой стоят засечки с подписью.
  let {
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    label = '',
    showValue = false,
    valueText = '',
    marks = [],
    disabled = false,
    class: className = '',
    ...rest
  } = $props();

  const shown = $derived(valueText || String(value));
  const fraction = (mark) => (max > min ? Math.min(1, Math.max(0, (mark - min) / (max - min))) : 0);
  const at = $derived(fraction(Number(value)));
</script>

<label class="k-slider {className}">
  {#if label}
    <span class="k-slider__head"><span class="k-label">{label}</span></span>
  {/if}
  <span class="k-slider__rail" style="--k-at: {at}">
    <span class="k-slider__track" aria-hidden="true"><span class="k-slider__fill"></span></span>
    {#if showValue}<span class="k-slider__bubble k-slider__value" aria-hidden="true">{shown}</span>{/if}
  <input
    type="range"
    class="k-slider__input"
    bind:value
    {min}
    {max}
    {step}
    {disabled}
    aria-valuetext={valueText || undefined}
    {...rest}
  />
  </span>
  {#if marks.length}
    <span class="k-slider__marks" aria-hidden="true">
      {#each marks as mark (mark)}
        <span class="k-slider__mark" class:is-on={Number(value) === mark} style="--k-mark: {fraction(mark)}">{mark}</span>
      {/each}
    </span>
  {/if}
</label>
