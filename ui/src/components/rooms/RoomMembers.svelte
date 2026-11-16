<script>
  // Состав группы в окне «Новая группа»: по флажку кита на бота.
  // Настоящие <input type="checkbox" value="id"> остаются в DOM: app.js читает отмеченных
  // через `#room-members input:checked` и слушает change на контейнере.
  import { untrack } from 'svelte';
  import Checkbox from '../kit/Checkbox.svelte';

  let { bots = [], selected = [], empty = '' } = $props();

  // У флажка есть значение по умолчанию, поэтому у каждого бота в chosen сразу true или false.
  const initial = () => Object.fromEntries((bots ?? []).map((bot) => [bot.id, (selected ?? []).includes(bot.id)]));
  let chosen = $state(untrack(initial));
  $effect(() => {
    chosen = initial();
  });

  const labelOf = (bot) => (bot.role ? `${bot.name} · ${bot.role}` : bot.name);
</script>

{#if bots.length}
  <ul class="rm-list">
    {#each bots as bot (bot.id)}
      <li><Checkbox value={bot.id} label={labelOf(bot)} bind:checked={chosen[bot.id]} /></li>
    {/each}
  </ul>
{:else}
  <p class="rm-empty">{empty}</p>
{/if}

<style>
  .rm-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--k-space-1) var(--k-space-3);
    list-style: none;
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface);
  }
  /* Скрытый input флажка абсолютный: без опоры он уезжает за край окна и даёт горизонтальную прокрутку. */
  .rm-list :global(.k-check) { position: relative; }
  .rm-list li { padding: var(--k-space-2) 0; }
  .rm-list li + li { border-top: 1px solid var(--k-line); }
  .rm-empty {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body);
  }
</style>
