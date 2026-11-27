<script>
  // Вкладка «Рутины» окна «Задачи и автоматизация»: список карточек и призрачная кнопка «Новая рутина».
  import { tr } from '../../lib/i18n.js';
  import { Button } from '../kit';
  import RoutineCard from './RoutineCard.svelte';

  /** routines: готовые модели карточек (см. RoutineCard); onnew() открывает форму новой рутины; lead и empty — подписи над списком и вместо него (в панели бота свои). */
  let { routines = [], lead = tr('Бот сам просыпается по расписанию и делает работу. Результат появится в его чате.'), empty = tr('Пока ни одной. Добавь первую, и бот начнёт работать сам.'), onrun, onresume, onunpause, onquiet, ontoggle, ondelete, onnew } = $props();
</script>

<div class="rp">
  {#if lead}<p class="rp-lead">{lead}</p>{/if}
  {#each routines as routine (routine.id)}
    <RoutineCard {routine} {onrun} {onresume} {onunpause} {onquiet} {ontoggle} {ondelete} />
  {:else}
    <p class="rp-empty">{empty}</p>
  {/each}
  <Button class="rp-new" variant="ghost" iconName="plus" onclick={() => onnew?.()}>{tr('Новая рутина')}</Button>
</div>

<style>
  .rp {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
  }
  .rp-lead,
  .rp-empty {
    margin: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    color: var(--k-text-3);
  }
  .rp :global(.rp-new) {
    align-self: flex-start;
  }
</style>
