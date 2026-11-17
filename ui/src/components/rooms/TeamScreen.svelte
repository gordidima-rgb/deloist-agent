<script>
  // Экран команды: шапка с картинкой, вкладки «Структура» и «Чат», схема «Ты → ведущий → участники».
  // Чат остаётся старым экраном app.js: этот компонент рисует только вкладки и схему, а при вкладке
  // «Чат» занимает одну полосу с вкладками. Данные и переходы держит app.js (props и колбэки).
  import Tabs from '../kit/Tabs.svelte';
  import Button from '../kit/Button.svelte';
  import Icon from '../kit/Icon.svelte';
  import { tr } from '../../lib/i18n.js';

  let {
    room = null,
    members = [],
    tab = 'structure',
    roomIcon = '',
    paint,
    ontab,
    onopenbot,
    onedit,
  } = $props();

  const items = $derived([
    { id: 'structure', label: tr('Структура') },
    { id: 'chat', label: tr('Чат') },
  ]);

  const lead = $derived(members.find((m) => m.id === room?.lead) ?? members[0] ?? null);
  const others = $derived(members.filter((m) => m.id !== lead?.id));

  const modeText = $derived(room?.mode === 'auto' ? tr('Модель выбирает, кто ответит') : tr('Отвечает ведущий, остальных зовёшь через @имя'));
  const rules = $derived((room?.instructions ?? '').trim());
  const rulesShort = $derived(rules.length > 220 ? `${rules.slice(0, 220).trimEnd()}…` : rules);

  // Живой статус бота: что ждёт тебя важнее всего, потом ошибка, потом работа.
  function state(member) {
    if (member.needs) return { key: 'needs', text: tr('Нужен ты') };
    if (member.error) return { key: 'error', text: tr('Ошибка') };
    if (member.busy) return { key: 'busy', text: tr('Работает') };
    return { key: 'idle', text: tr('Свободен') };
  }

  // Одна функция на место аватара: новая в {@attach} пересоздала бы строку (как в BotList).
  const attachers = new Map();
  const avatarOf = (id) => {
    const key = `${room?.id}|${id}`;
    if (!attachers.has(key)) attachers.set(key, (el) => paint?.(el, id, `room:${room?.id}:${id}`));
    return attachers.get(key);
  };
</script>

<div class="ts" class:is-structure={tab === 'structure'}>
  <div class="ts-bar">
    <Tabs {items} value={tab} label={tr('Экран команды')} onchange={(id) => ontab?.(id)} />
  </div>

  {#if tab === 'structure' && room}
    <div class="ts-body">
      <span class="ts-pic" style={room.color ? `--team-color: ${room.color}` : undefined} role="img" aria-label={room.name}>
        {#if room.pictureUrl}
          <img src={room.pictureUrl} alt="" />
        {:else if room.emoji}
          {room.emoji}
        {:else}
          {@html roomIcon}
        {/if}
      </span>
      <div class="ts-chart" role="tree" aria-label={tr('Структура команды')}>
        <div class="ts-card ts-you" role="treeitem" aria-level="1" aria-selected="false">
          <span class="ts-you-mark" aria-hidden="true"><Icon name="user" size={18} /></span>
          <span class="ts-text">
            <span class="ts-title">{tr('Ты')}</span>
            <span class="ts-sub">{tr('ставишь задачи и одобряешь')}</span>
          </span>
        </div>

        <div class="ts-link" aria-hidden="true"></div>
        {#if lead}
          {@render botCard(lead, true)}
        {:else}
          <p class="ts-empty">{tr('В команде пока нет ботов.')}</p>
        {/if}

        {#if others.length}
          <div class="ts-link" aria-hidden="true"></div>
          <ul class="ts-kids" role="group">
            {#each others as member (member.id)}
              <li class="ts-kid">{@render botCard(member, false)}</li>
            {/each}
          </ul>
        {/if}
      </div>

      <section class="ts-about" aria-label={tr('О команде')}>
        <div class="ts-about-row">
          <span class="ts-about-label">{tr('Как отвечает')}</span>
          <span class="ts-about-text">{modeText}</span>
        </div>
        <div class="ts-about-row">
          <span class="ts-about-label">{tr('Правила')}</span>
          <span class="ts-about-text" class:is-quiet={!rulesShort}>{rulesShort || tr('Правил нет: каждый бот работает по своей роли.')}</span>
        </div>
        <div class="ts-about-actions">
          <Button size="sm" onclick={() => onedit?.()}>
            {#snippet icon()}<Icon name="pencil" size={14} />{/snippet}
            {tr('Изменить команду')}
          </Button>
        </div>
      </section>
    </div>
  {/if}
</div>

{#snippet botCard(member, isLead)}
  {@const st = state(member)}
  <button
    type="button"
    class="ts-card ts-bot is-{st.key}"
    class:is-lead={isLead}
    data-bot={member.id}
    role="treeitem"
    aria-level={isLead ? 2 : 3}
    aria-selected="false"
    onclick={() => onopenbot?.(member.id)}
  >
    <span class="avatar ts-avatar" {@attach avatarOf(member.id)}></span>
    <span class="ts-text">
      <span class="ts-title">{member.name}</span>
      <span class="ts-sub">{member.role || tr('без роли')}</span>
      <span class="ts-state is-{st.key}"><span class="ts-dot" aria-hidden="true"></span>{st.text}</span>
    </span>
    {#if isLead}<span class="ts-badge">{tr('Ведущий')}</span>{/if}
  </button>
{/snippet}

<style>
  .ts {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .ts.is-structure {
    flex: 1;
  }
  .ts-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--k-line);
    background: var(--k-canvas);
    min-width: 0;
  }
  .ts-bar :global(.k-tabs) {
    flex: none;
  }
  .ts-pic {
    flex: none;
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    border-radius: 26px;
    overflow: hidden;
    background: var(--k-surface-2);
    color: var(--team-color, var(--k-text-2));
    font-size: 34px;
    line-height: 1;
    margin-bottom: -8px;
  }
  .ts-pic img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .ts-pic :global(svg) {
    width: 32px;
    height: 32px;
  }

  .ts-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 24px 16px 32px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 24px;
  }
  .ts-chart {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
    max-width: 880px;
  }
  .ts-link {
    width: 1px;
    height: 20px;
    background: var(--k-line-strong);
  }

  .ts-card {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    box-sizing: border-box;
    width: 232px;
    max-width: 100%;
    padding: 12px;
    border: 1px solid var(--k-line);
    border-radius: 20px;
    background: var(--k-surface);
    color: var(--k-text);
    font: inherit;
    text-align: left;
    box-shadow: var(--k-shadow-1);
  }
  button.ts-card {
    cursor: pointer;
    transition: box-shadow 0.15s ease, border-color 0.15s ease;
  }
  button.ts-card:hover {
    box-shadow: var(--k-shadow-2);
    border-color: var(--k-line-strong);
  }
  button.ts-card:focus-visible {
    outline: 2px solid var(--k-brand);
    outline-offset: 2px;
  }
  .ts-bot.is-needs {
    border-color: var(--k-attention);
  }
  .ts-you {
    background: var(--k-surface-2);
    box-shadow: none;
  }
  .ts-you-mark {
    flex: none;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 999px;
    background: var(--k-ink);
    color: var(--k-on-ink);
  }
  .ts-avatar {
    flex: none;
    width: 40px;
    height: 40px;
  }
  .ts-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ts-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ts-sub {
    color: var(--k-text-3);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ts-state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    font-size: 12px;
    color: var(--k-text-2);
  }
  .ts-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--k-line-strong);
  }
  .ts-state.is-busy .ts-dot {
    background: var(--k-ai);
  }
  .ts-state.is-needs,
  .ts-state.is-error {
    color: var(--k-attention);
  }
  .ts-state.is-needs .ts-dot,
  .ts-state.is-error .ts-dot {
    background: var(--k-attention);
  }
  .ts-badge {
    position: absolute;
    top: -9px;
    right: 14px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--k-ink);
    color: var(--k-on-ink);
    font-size: 11px;
    font-weight: 600;
  }
  .ts-empty {
    margin: 0;
    color: var(--k-text-3);
  }

  /* Участники: ряд карточек, к каждой своя ветка от общей шины. */
  .ts-kids {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 0 16px;
    margin: 0;
    padding: 0;
    list-style: none;
    width: 100%;
  }
  .ts-kid {
    position: relative;
    padding-top: 20px;
    margin-bottom: 4px;
    display: flex;
    justify-content: center;
  }
  .ts-kid::before {
    content: '';
    position: absolute;
    top: 0;
    left: 50%;
    width: 1px;
    height: 20px;
    background: var(--k-line-strong);
  }
  .ts-kid::after {
    content: '';
    position: absolute;
    top: 0;
    left: -8px;
    right: -8px;
    height: 1px;
    background: var(--k-line-strong);
  }
  .ts-kid:first-child::after {
    left: 50%;
  }
  .ts-kid:last-child::after {
    right: 50%;
  }
  .ts-kid:only-child::after {
    display: none;
  }

  .ts-about {
    width: 100%;
    max-width: 640px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--k-line);
    border-radius: 20px;
    background: var(--k-surface);
  }
  .ts-about-row {
    display: flex;
    gap: 12px;
    align-items: baseline;
  }
  .ts-about-label {
    flex: none;
    width: 112px;
    color: var(--k-text-3);
    font-size: 12px;
  }
  .ts-about-text {
    min-width: 0;
    color: var(--k-text);
    overflow-wrap: anywhere;
    white-space: pre-line;
  }
  .ts-about-text.is-quiet {
    color: var(--k-text-3);
  }
  .ts-about-actions {
    display: flex;
    justify-content: flex-end;
  }

  /* Узкое окно: участники в столбик, ветки идут от левой рейки. */
  @media (max-width: 640px) {
    .ts-chart {
      align-items: stretch;
    }
    .ts-card {
      width: auto;
    }
    .ts-link {
      margin-left: 24px;
    }
    .ts-kids {
      flex-direction: column;
      flex-wrap: nowrap;
      gap: 12px;
      margin-left: 24px;
      width: auto;
      align-items: stretch;
      padding-left: 20px;
      border-left: 1px solid var(--k-line-strong);
    }
    .ts-kid {
      padding-top: 0;
      margin-bottom: 0;
    }
    .ts-kid :global(.ts-card) {
      width: 100%;
    }
    .ts-kid::before {
      top: 50%;
      left: -20px;
      width: 20px;
      height: 1px;
    }
    .ts-kid::after {
      display: none;
    }
    .ts-about-row {
      flex-direction: column;
      gap: 2px;
    }
    .ts-about-label {
      width: auto;
    }
  }
</style>
