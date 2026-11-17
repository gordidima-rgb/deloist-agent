<script>
  // "Your skills" at the top of the Skills page: what the person already has, from any origin.
  import { tr } from '../../lib/i18n.js';

  let { skills = [], bots = [], onopen } = $props();

  const LIMIT = 6;
  let all = $state(false);

  const usage = $derived.by(() => {
    const out = new Map();
    for (const bot of bots) for (const id of bot.skills || []) out.set(id, (out.get(id) || 0) + 1);
    return out;
  });
  const shown = $derived(all ? skills : skills.slice(0, LIMIT));

  function origin(skill) {
    if (skill.catalogId) return tr('Из каталога');
    if (skill.origin === 'learned') return tr('Выучил бот');
    return tr('Свой');
  }
</script>

<section class="yours" aria-labelledby="your-skills">
  <div class="head">
    <h2 id="your-skills">{tr('Ваши навыки')}</h2>
    {#if skills.length > LIMIT}
      <button type="button" class="k-btn k-btn--ghost k-btn--sm" onclick={() => (all = !all)}>
        {all ? tr('Свернуть') : tr('Показать все ({count})', { count: skills.length })}
      </button>
    {/if}
  </div>
  {#if skills.length === 0}
    <p class="none">{tr('Своих навыков пока нет. Добавьте готовый ниже или попросите бота запомнить, как он сделал работу.')}</p>
  {:else}
    <ul class="list">
      {#each shown as skill (skill.id)}
        <li>
          <button
            type="button"
            class="skill"
            disabled={!skill.catalogId}
            onclick={() => skill.catalogId && onopen?.(skill.catalogId)}
            title={skill.description || skill.name}
          >
            <span class="name">{skill.name}</span>
            <span class="meta">
              <span class="tag">{origin(skill)}</span>
              {tr('ботов: {count}', { count: usage.get(skill.id) || 0 })}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .yours {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font: 800 var(--k-text-lg, 18px) / 1.2 var(--k-font-display, inherit);
  }
  .none {
    margin: 0;
    color: var(--k-text-2, inherit);
    font-size: var(--k-text-sm, 13px);
  }
  .list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--k-space-2, 8px);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .skill {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: var(--k-space-2, 8px) var(--k-space-3, 12px);
    border: 0;
    border-radius: var(--k-radius-md, 18px);
    background: var(--k-surface);
    color: var(--k-text, inherit);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .skill:disabled {
    cursor: default;
  }
  .skill:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
  .name {
    font-weight: 600;
    font-size: var(--k-text-sm, 13px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--k-text-3, inherit);
    font-size: var(--k-text-xs, 12px);
  }
  .tag {
    color: var(--k-text-2, inherit);
    font-weight: 600;
  }
</style>
