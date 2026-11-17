<script>
  import ListRow from './ListRow.svelte';
  let { variant = 'plain', actionLabel = '', onaction, onchange } = $props();
</script>

<ul class="k-lrows">
  <ListRow icon="link" title="Row" sub="Sub" {variant} {actionLabel} {onaction} {onchange}>
    {#snippet children()}<p>Body</p>{/snippet}
  </ListRow>
</ul>
