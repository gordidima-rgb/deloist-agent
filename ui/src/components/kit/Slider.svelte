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
  const fraction = (mark) => (max > min ? (mark - min) / (max - min) : 0);
</script>

<label class="k-slider {className}">
  {#if label || showValue}
    <span class="k-slider__head">
      {#if label}<span class="k-label">{label}</span>{/if}
      {#if showValue}<span class="k-slider__value">{shown}</span>{/if}
    </span>
  {/if}
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
  {#if marks.length}
    <span class="k-slider__marks" aria-hidden="true">
      {#each marks as mark (mark)}
        <span class="k-slider__mark" style="--k-mark: {fraction(mark)}">{mark}</span>
      {/each}
    </span>
  {/if}
</label>
