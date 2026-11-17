<script>
  // Строка ввода на .k-input. id и describedby приходят из Field; invalid включает коралловую рамку.
  let {
    value = $bindable(''),
    type = 'text',
    id = '',
    describedby = '',
    invalid = false,
    disabled = false,
    class: className = '',
    ...rest
  } = $props();
</script>

<input
  {type}
  {id}
  bind:value
  class="k-input {className}"
  aria-invalid={invalid ? 'true' : undefined}
  aria-describedby={describedby || undefined}
  {disabled}
  {...rest}
/>
