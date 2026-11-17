<script>
  import { Button } from './kit';
  import { tr } from '../lib/i18n.js';
  let { held = false, busy = false, error = '', ontoggle } = $props();
</script>

<div class="screen-control">
  {#if held}<span class="screen-control-status">{tr('Управление у тебя')}</span>{/if}
  <Button variant="primary" size="sm" loading={busy} onclick={() => ontoggle?.()}>
    {busy ? tr('Подождите…') : held ? tr('Вернуть боту') : tr('Взять управление')}
  </Button>
  {#if error}<span class="screen-control-error" role="alert">{error}</span>{/if}
</div>

<style>
  .screen-control { position:absolute; z-index:2; bottom:var(--k-space-2,8px); left:var(--k-space-2,8px); display:flex; align-items:center; flex-wrap:wrap; gap:var(--k-space-2,8px); max-width:calc(100% - 16px); }
  .screen-control-status,.screen-control-error { padding: var(--k-space-1) var(--k-space-2); border-radius:var(--k-radius-pill,999px); background:var(--k-ink); color:var(--k-on-ink); font-size:var(--k-text-sm,13px); }
  .screen-control-error { background:var(--k-attention-text); }
</style>
