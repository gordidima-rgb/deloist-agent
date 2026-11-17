<script>
  // Карточка «Нужно разрешение» (.k-approval, docs/design/kit.md): единственная громкая карточка чата.
  // Данные ей даёт web/app.js (approvalCard), решение уходит обратно через onanswer(answer):
  // сервер получает только ответ, что именно разрешается, он берёт из своей карточки.
  import Button from '../kit/Button.svelte';

  // actions: [{ answer, label, variant? }] по порядку; resolved: подпись после решения.
  // Человеческая карточка: title — короткая фраза, которую собрала программа; risk — low | medium | high (считает программа);
  // purpose — слова самого бота (только показ, на решение не влияют); details — технические подробности (свёрнуты).
  let { title, what = '', scope = '', kind = '', actions = [], danger = false, busy = false, resolved = '', onanswer, risk = '', riskLabel = '', purpose = '', purposeLabel = '', details = '', detailsLabel = '' } = $props();

  const firstAnswer = $derived(actions[0]?.answer);
</script>

<section class="k-approval k-sharpen-in" class:is-resolved={!!resolved} aria-label={title} data-approval>
  <div class="k-approval__head">
    <span class="k-pix k-pix--paper k-pix--wait" role="img" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
    <h3 class="k-approval__title">{title}</h3>
  </div>
  {#if risk}<div class="k-approval__risk" data-risk={risk}>{riskLabel}</div>{/if}
  {#if purpose}<p class="k-approval__purpose"><strong>{purposeLabel}</strong> {purpose}</p>{/if}
  {#if what}<pre class="k-approval__what">{what}</pre>{/if}
  {#if details}
    <details class="k-approval__details">
      <summary>{detailsLabel}</summary>
      <pre class="k-approval__raw">{details}</pre>
    </details>
  {/if}
  {#if scope}
    <div class="k-approval__scope">{scope}{#if kind}<span class="k-approval__kind">{kind}</span>{/if}</div>
  {/if}
  <div class="k-approval__actions">
    {#if resolved}
      <span class="k-approval__resolved">{resolved}</span>
    {:else}
      {#each actions as action (action.answer)}
        <Button
          variant={action.answer === 'deny' ? 'ghost' : action.answer === firstAnswer ? (danger ? 'attention' : 'primary') : 'secondary'}
          size="sm"
          disabled={busy}
          data-answer={action.answer}
          onclick={() => onanswer?.(action.answer)}
        >{action.label}</Button>
      {/each}
    {/if}
  </div>
</section>

<style>
  .k-approval__head { align-items: center; }
  h3.k-approval__title { margin: 0; }
  pre.k-approval__what { margin: 0; white-space: pre-wrap; max-height: 220px; overflow: auto; }
  .k-approval__risk { align-self: flex-start; padding: 1px 8px; border-radius: var(--k-radius-pill); background: var(--k-surface-2); color: var(--k-text-2); font: 600 var(--k-text-xs) / 1.4 var(--k-font-body); }
  .k-approval__risk[data-risk='high'] { background: var(--k-coral-soft, var(--k-surface-2)); color: var(--k-coral, var(--k-text)); }
  p.k-approval__purpose { margin: 0; font: 400 var(--k-text-sm) / 1.45 var(--k-font-body); color: var(--k-text-2); overflow-wrap: anywhere; }
  .k-approval__details summary { cursor: pointer; font: 600 var(--k-text-xs) / 1.4 var(--k-font-body); color: var(--k-text-3); }
  pre.k-approval__raw { margin: var(--k-space-2) 0 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 220px; overflow: auto; font: 400 var(--k-text-xs) / 1.45 var(--k-font-mono, monospace); }
  .k-approval__scope { display: flex; align-items: center; gap: var(--k-space-2); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); color: var(--k-text-3); }
  .k-approval__kind { padding: 1px 8px; border-radius: var(--k-radius-pill); background: var(--k-surface-2); color: var(--k-text-2); font-weight: 600; }
  .k-approval__resolved { font: 600 var(--k-text-sm) / 1.4 var(--k-font-body); color: var(--k-text-3); }
  /* Решено: тихая карточка без коралловой тени, остаётся в переписке как запись. */
  .is-resolved { box-shadow: var(--k-shadow-1); }
  .is-resolved :global(.k-pix) { display: none; }
</style>
