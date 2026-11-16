<script>
  import { tr } from '../lib/i18n.js';
  let { held = false, busy = false, error = '', ontoggle } = $props();
</script>

<div class="screen-control">
  {#if held}<span class="screen-control-status">{tr('Управление у тебя')}</span>{/if}
  <button type="button" disabled={busy} onclick={() => ontoggle?.()}>
    {busy ? tr('Подождите…') : held ? tr('Вернуть боту') : tr('Взять управление')}
  </button>
  {#if error}<span class="screen-control-error" role="alert">{error}</span>{/if}
</div>

<style>
  .screen-control { position:absolute; z-index:2; bottom:var(--k-space-2,8px); left:var(--k-space-2,8px); display:flex; align-items:center; flex-wrap:wrap; gap:var(--k-space-2,8px); max-width:calc(100% - 16px); }
  button { border:0; border-radius:var(--k-radius-pill,999px); padding:8px 14px; background:var(--k-brand,#025bff); color:var(--k-on-brand,#fff); font:inherit; font-weight:600; cursor:pointer; box-shadow:0 2px 10px #0008; }
  button:disabled { opacity:.7; cursor:wait; }
  button:focus-visible { outline:2px solid white; outline-offset:2px; }
  .screen-control-status,.screen-control-error { padding:5px 9px; border-radius:var(--k-radius-pill,999px); background:var(--k-ink,#000); color:var(--k-on-ink,#fff); font-size:var(--k-text-sm,13px); }
  .screen-control-error { background:var(--k-attention-text,#b4321b); }
</style>
