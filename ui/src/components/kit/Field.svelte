<script module>
  let seq = 0;
</script>

<script>
  // Обёртка поля: метка, подсказка или ошибка и сам элемент управления.
  // children получает { id, describedby, invalid }, чтобы связать input с текстом подсказки.
  let {
    label = '',
    hint = '',
    error = '',
    for: htmlFor = '',
    class: className = '',
    children,
    ...rest
  } = $props();

  const uid = `k-field-${++seq}`;
  const id = $derived(htmlFor || uid);
  const hintId = $derived(`${id}-hint`);
  const describedby = $derived(error || hint ? hintId : undefined);
  const invalid = $derived(Boolean(error));
</script>

<div class="k-field {className}" {...rest}>
  {#if label}<label class="k-label" for={id}>{label}</label>{/if}
  {@render children?.({ id, describedby, invalid })}
  {#if error}
    <p class="k-hint is-error" id={hintId}>{error}</p>
  {:else if hint}
    <p class="k-hint" id={hintId}>{hint}</p>
  {/if}
</div>
