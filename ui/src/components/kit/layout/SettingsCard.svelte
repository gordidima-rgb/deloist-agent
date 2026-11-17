<script>
  // Раздел формы настроек: карточка с плиткой-значком, заголовком и необязательным серым пояснением.
  // Лежит внутри .k-sform. Значок — имя из lib/icons.js. Строки полей — FieldRow.
  import Icon from '../media/Icon.svelte';

  let { title = '', description = '', icon = '', class: className = '', actions = null, children, ...rest } = $props();
</script>

<section class="k-scard {className}" {...rest}>
  {#if title}
    <header class="k-scard__head">
      {#if icon}<span class="k-tile" aria-hidden="true"><Icon name={icon} size={null} /></span>{/if}
      <div class="k-scard__text">
        <h2 class="k-scard__title">{title}</h2>
        {#if description}<p class="k-scard__desc">{description}</p>{/if}
      </div>
      {#if actions}{@render actions()}{/if}
    </header>
  {/if}
  {@render children?.()}
</section>
