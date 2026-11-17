<script>
  // Выбор модели в окне «Новый бот»: список уже на виду, один клик по карточке — и модель выбрана.
  // Первой стоит «как в настройках» (общая модель программы), дальше модели подключённых провайдеров;
  // остальные открывает «Ещё» — тот же ModelPicker, что и в профиле бота. Выбор отдаёт { driver, model }.
  import { tr } from '../lib/i18n.js';
  import { buildGroups, toValue, isCurrent, iconSvg } from '../lib/modelPicker.js';
  import { Chip } from './kit';
  import ModelPicker from './ModelPicker.svelte';

  let { providers = [], plan = null, settings = {}, value = { driver: '', model: '' }, onselect, onconnect } = $props();

  const PER_GROUP = 3;
  const MAX_CHIPS = 7;

  let moreOpen = $state(false);
  let moreBtn = $state(null);

  const groups = $derived(buildGroups({ providers, plan, settings, value }));

  // Карточки: по несколько первых моделей каждого провайдера. Выбранная модель видна всегда,
  // даже если она из «Ещё». Строки без своей модели («Модель из настроек») в карточки не берём.
  const chips = $derived.by(() => {
    const out = [];
    for (const g of groups) {
      if (g.kind === 'manual' || g.kind === 'missing') continue;
      for (const m of g.models.filter((x) => x.id !== '' || g.kind === 'demo').slice(0, PER_GROUP)) out.push({ g, m });
    }
    const head = out.slice(0, MAX_CHIPS);
    if (value.driver && !head.some(({ g, m }) => isCurrent(g, m, value))) {
      const hit = out.find(({ g, m }) => isCurrent(g, m, value));
      if (hit) head[head.length - 1] = hit;
    }
    return head;
  });
  const hidden = $derived(Math.max(0, groups.reduce((n, g) => n + g.models.length, 0) - chips.length));

  const nameOf = (g, m) => (g.kind === 'demo' ? g.name : m.name);

  function pickDefault() {
    onselect?.({ driver: '', model: '' });
  }
  function pick(g, m) {
    onselect?.(toValue(g, m));
  }
</script>

<div class="nbm">
  <span class="nbm-label" id="nbm-label">{tr('Модель')}</span>
  <div class="nbm-list" role="radiogroup" aria-labelledby="nbm-label">
    <Chip class="nbm-chip" bind:selected={() => !value.driver, () => {}} role="radio" aria-checked={!value.driver} onclick={pickDefault}>
      {tr('как в настройках')}
    </Chip>
    {#each chips as { g, m } (g.id + '/' + m.id)}
      {@const on = isCurrent(g, m, value)}
      {@const svg = iconSvg(g)}
      <Chip class="nbm-chip" bind:selected={() => on, () => {}} role="radio" aria-checked={on} title={g.name} onclick={() => pick(g, m)}>
        {#if svg}<span class="nbm-logo" aria-hidden="true">{@html svg}</span>{/if}
        <span class="nbm-name">{nameOf(g, m)}</span>
      </Chip>
    {/each}
    <Chip class="nbm-more" bind:ref={moreBtn} bind:selected={() => false, () => {}} aria-haspopup="dialog" aria-expanded={moreOpen} onclick={() => (moreOpen = !moreOpen)}>
      {hidden > 0 ? tr('Ещё · {n}', { n: hidden }) : tr('Ещё…')}
    </Chip>
  </div>
  {#if moreOpen}
    <ModelPicker
      startOpen
      bare
      alignLeft
      anchor={moreBtn}
      {providers}
      {plan}
      {settings}
      {value}
      onselect={(next) => {
        moreOpen = false;
        onselect?.(next);
      }}
      {onconnect}
      onclose={() => (moreOpen = false)}
    />
  {/if}
</div>

<style>
  .nbm {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .nbm-label {
    color: var(--k-text-2);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
  }
  .nbm-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .nbm-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nbm-logo {
    flex: none;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
  }
  .nbm-logo :global(svg) {
    width: 100%;
    height: 100%;
  }
  :global(.nbm-more) {
    color: var(--k-text-3);
    border-style: dashed;
  }
</style>
