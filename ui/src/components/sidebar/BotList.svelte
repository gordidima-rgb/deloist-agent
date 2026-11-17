<script>
  // Список слева: закреплённые, дерево команд (команда → ведущий → остальные), боты без команды,
  // специалисты «в запасе» (свёрнуты) и архив команд.
  // Данные и выбор держит app.js; аватар рисует его же движок через paint(el, id, slot),
  // потому что живые аватары кэшируются по месту и переезжают между строками. Бот в двух командах
  // получает по своему месту (slot) на каждую строку, иначе аватар ушёл бы из первой.
  import ListItem from '../kit/ListItem.svelte';
  import { tr } from '../../lib/i18n.js';
  import { plainPreview, teamSubtitle } from '../../lib/preview.js';

  import Icon from '../kit/Icon.svelte';
  let {
    bots = [],
    rooms = [],
    filter = '',
    currentId = '',
    roomIcon = '',
    paint,
    onselect,
    onselectroom,
    onpin,
    // Меню бота: кнопка «Настройки» в строке и правый щелчок. (id, { x, y, trigger }) — точка, где открыть.
    onmenu,
  } = $props();

  // from — команда, в ветке которой стоит строка: «Перенести» уводит бота именно из неё.
  function openMenu(event, id, from = '') {
    event.preventDefault();
    event.stopPropagation();
    const trigger = event.type === 'contextmenu' ? null : event.currentTarget;
    const box = trigger?.getBoundingClientRect();
    onmenu?.(id, box ? { x: box.right, y: box.bottom, trigger, from } : { x: event.clientX, y: event.clientY, trigger: null, from });
  }

  const RESERVE_KEY = 'deloist:sidebar-reserve-open';
  const ARCHIVE_KEY = 'deloist:sidebar-archive-open';
  const TEAMS_KEY = 'deloist:sidebar-teams-closed';
  const readOpen = (key) => {
    try {
      return localStorage.getItem(key) === '1';
    } catch {
      return false;
    }
  };
  const rememberOpen = (key, open) => {
    try {
      localStorage.setItem(key, open ? '1' : '0');
    } catch {
      /* без хранилища раздел просто не запомнится */
    }
  };
  // Свёрнутые команды помним списком id: по умолчанию команда развёрнута.
  const readClosed = () => {
    try {
      const list = JSON.parse(localStorage.getItem(TEAMS_KEY) || '[]');
      return Object.fromEntries((Array.isArray(list) ? list : []).map((id) => [String(id), true]));
    } catch {
      return {};
    }
  };
  let reserveOpen = $state(readOpen(RESERVE_KEY));
  let archiveOpen = $state(readOpen(ARCHIVE_KEY));
  let closed = $state(readClosed());
  const toggleReserve = () => {
    reserveOpen = !reserveOpen;
    rememberOpen(RESERVE_KEY, reserveOpen);
  };
  const toggleArchive = () => {
    archiveOpen = !archiveOpen;
    rememberOpen(ARCHIVE_KEY, archiveOpen);
  };
  function setTeamOpen(id, open) {
    closed = { ...closed, [id]: !open };
    try {
      localStorage.setItem(TEAMS_KEY, JSON.stringify(Object.keys(closed).filter((key) => closed[key])));
    } catch {
      /* без хранилища команда просто не запомнится */
    }
  }

  const searching = $derived(filter.trim() !== '');
  // Поиск раскрывает команды: совпавшего бота не прячет свёрнутая ветка.
  const isOpen = (id) => searching || !closed[id];

  // Специалист, который сейчас работает, ждёт ответа или открыт, виден среди старших.
  const inMain = (bot) => bot.rank !== 'specialist' || bot.busy || bot.unread || bot.id === currentId;
  const byId = $derived(new Map(bots.map((b) => [b.id, b])));
  // Архив — отдельный свёрнутый раздел в конце: законченные команды не спорят с живыми.
  const liveRooms = $derived(rooms.filter((room) => !room.archived));
  const archivedRooms = $derived(rooms.filter((room) => room.archived));

  // Дерево: команда → ведущий (Room.lead, иначе первый участник) → остальные на уровень ниже.
  // Бот не повторяется в одной команде; во второй и далее он помечен как повтор.
  const teams = $derived.by(() => {
    const seen = new Set();
    return liveRooms.map((room) => {
      const ids = [...new Set((room.memberIds ?? []).filter((id) => byId.has(id)))];
      const leadId = ids.includes(room.lead) ? room.lead : ids[0];
      const mark = (id) => {
        const repeat = seen.has(id);
        seen.add(id);
        return { bot: byId.get(id), repeat };
      };
      const lead = leadId ? mark(leadId) : null;
      const others = ids.filter((id) => id !== leadId).map(mark);
      const members = ids.map((id) => byId.get(id));
      return {
        room,
        lead,
        others,
        count: members.length,
        subtitle: teamSubtitle(room.preview, members.map((b) => b.name), tr('пока пусто')),
        needs: members.filter((b) => b.needs).length,
        errors: members.filter((b) => b.error).length,
        unread: members.filter((b) => b.unread).length,
      };
    });
  });
  const roomSubtitle = (room) => teamSubtitle(room.preview, (room.memberIds ?? []).map((id) => byId.get(id)?.name), tr('пока пусто'));
  const inTeam = $derived(new Set(teams.flatMap((t) => [t.lead?.bot.id, ...t.others.map((o) => o.bot.id)].filter(Boolean))));

  const pinned = $derived(searching ? [] : bots.filter((b) => b.pinned));
  const free = $derived(bots.filter((b) => !inTeam.has(b.id)));
  const main = $derived(searching ? free : free.filter((b) => !b.pinned && inMain(b)));
  const reserve = $derived(searching ? [] : free.filter((b) => !b.pinned && !inMain(b)));

  // Одна функция на место аватара: новая функция в {@attach} пересоздала бы строку.
  const attachers = new Map();
  const avatarOf = (id, slot) => {
    const key = `${id}|${slot}`;
    if (!attachers.has(key)) attachers.set(key, (el) => paint?.(el, id, slot));
    return attachers.get(key);
  };

  // Стрелки по дереву: вниз и вверх по видимым строкам, вправо раскрывает или идёт внутрь,
  // влево сворачивает или идёт к команде. Enter и щелчок открывают чат (родная кнопка).
  function visibleRows(tree) {
    return [...tree.querySelectorAll('.k-list-item__main')].filter((el) => !el.closest('[inert]'));
  }
  function onTreeKey(event) {
    const tree = event.currentTarget;
    const main = event.target.closest?.('.k-list-item__main');
    if (!main) return;
    const row = main.closest('li');
    const rows = visibleRows(tree);
    const at = rows.indexOf(main);
    const focus = (el) => {
      event.preventDefault();
      el?.focus({ preventScroll: false });
    };
    const team = row.dataset.team;
    if (event.key === 'ArrowDown') focus(rows[Math.min(at + 1, rows.length - 1)]);
    else if (event.key === 'ArrowUp') focus(rows[Math.max(at - 1, 0)]);
    else if (event.key === 'Home') focus(rows[0]);
    else if (event.key === 'End') focus(rows[rows.length - 1]);
    else if (event.key === 'ArrowRight' && team && row.hasAttribute('aria-expanded')) {
      if (row.getAttribute('aria-expanded') === 'false') {
        event.preventDefault();
        setTeamOpen(team, true);
      } else focus(rows[at + 1]);
    } else if (event.key === 'ArrowLeft') {
      if (team && row.getAttribute('aria-expanded') === 'true' && !searching) {
        event.preventDefault();
        setTeamOpen(team, false);
      } else if (row.dataset.parent) {
        focus(tree.querySelector(`li[data-team="${CSS.escape(row.dataset.parent)}"] .k-list-item__main`));
      }
    }
  }
</script>

{#snippet botRow(bot, opts = {})}
  {@const slot = opts.slot ?? `side:${bot.id}`}
  <ListItem
    class={opts.repeat ? 'sb-row is-repeat' : 'sb-row'}
    data-id={bot.id}
    data-parent={opts.parent}
    role={opts.level ? 'treeitem' : undefined}
    aria-level={opts.level}
    title={bot.name}
    aria-description={opts.repeat ? tr('есть и в другой команде') : undefined}
    subtitle={plainPreview(bot.preview)}
    meta={bot.time}
    alignMeta
    pinned={false}
    active={bot.id === currentId}
    unread={bot.unread}
    onclick={() => onselect?.(bot.id)}
    oncontextmenu={onmenu ? (event) => openMenu(event, bot.id, opts.parent) : undefined}
  >
    {#snippet avatar()}<span class="avatar sb-avatar" data-slot={slot} {@attach avatarOf(bot.id, slot)}></span>{/snippet}
    {#snippet context()}
      {#if onmenu}
        <button
          type="button"
          class="sb-pin sb-gear"
          title={tr('Настройки бота')}
          aria-label={tr('Настройки бота «{name}»', { name: bot.name })}
          aria-haspopup="menu"
          data-menu-trigger
          onclick={(event) => openMenu(event, bot.id, opts.parent)}
        >
          <Icon name="gear-simple" size={15} />
        </button>
      {/if}
      <button
        type="button"
        class="sb-pin"
        class:is-on={bot.pinned}
        title={bot.pinned ? tr('Открепить') : tr('Закрепить')}
        aria-label={bot.pinned ? tr('Открепить') : tr('Закрепить')}
        aria-pressed={bot.pinned}
        onclick={() => onpin?.(bot.id, !bot.pinned)}
      >
        <Icon name="pin" size={15} />
      </button>
    {/snippet}
  </ListItem>
{/snippet}

{#snippet roomRow(room)}
  <ListItem
    class="sb-row"
    data-id={room.id}
    title={room.name}
    subtitle={roomSubtitle(room)}
    meta={room.time}
    alignMeta
    active={room.active}
    unread={room.unread}
    onclick={() => onselectroom?.(room.id)}
  >
    {#snippet avatar()}<span class="avatar sb-avatar room-icon">{@html roomIcon}</span>{/snippet}
  </ListItem>
{/snippet}

{#snippet teamRow(team)}
  {@const room = team.room}
  {@const open = isOpen(room.id)}
  <ListItem
    class="sb-row sb-team"
    data-id={room.id}
    data-team={room.id}
    role="treeitem"
    aria-level="1"
    aria-expanded={team.count ? open : undefined}
    title={room.name}
    subtitle={team.subtitle}
    active={room.active}
    unread={room.unread}
    onclick={() => {
      onselectroom?.(room.id);
      if (!open && team.count) setTeamOpen(room.id, true);
    }}
  >
    {#snippet avatar()}
      <span class="sb-teamlead">
        {#if team.count}
          <button
            type="button"
            class="sb-chev-btn"
            tabindex="-1"
            aria-label={open ? tr('Свернуть команду «{name}»', { name: room.name }) : tr('Развернуть команду «{name}»', { name: room.name })}
            onclick={(event) => {
              event.stopPropagation();
              if (!searching) setTeamOpen(room.id, !open);
            }}
          >
            <Icon name="chevron-right" size={14} class={['sb-chev', open && 'is-open']} />
          </button>
        {/if}
        <span class="avatar sb-avatar room-icon" style={room.color ? `--team-color: ${room.color}` : undefined}>
          {#if room.pictureUrl}<img class="sb-teampic" src={room.pictureUrl} alt="" />{:else if room.emoji}{room.emoji}{:else}{@html roomIcon}{/if}
        </span>
      </span>
    {/snippet}
    {#snippet context()}
      {#if !open && team.needs}
        <span class="sb-roll is-needs" title={tr('Нужен ты: {n}', { n: team.needs })} aria-label={tr('Нужен ты: {n}', { n: team.needs })}>{team.needs}</span>
      {:else if !open && team.errors}
        <span class="sb-roll is-error" title={tr('Ошибка у участника')} aria-label={tr('Ошибка у участника')}>!</span>
      {/if}
      {#if !open && team.unread}
        <span class="sb-roll" title={tr('Непрочитанных ботов: {n}', { n: team.unread })} aria-label={tr('Непрочитанных ботов: {n}', { n: team.unread })}>{team.unread}</span>
      {/if}
      <span class="sb-members" title={tr('Участников: {n}', { n: team.count })}>{team.count}</span>
    {/snippet}
  </ListItem>
{/snippet}

{#if !bots.length && !rooms.length}
  <p class="sb-empty">{tr('Никого не нашлось.')}</p>
{:else}
  {#if pinned.length}
    <h3 class="sb-head">{tr('Закреплённые')}</h3>
    <ul class="k-list sb-list">
      {#each pinned as bot (bot.id)}{@render botRow(bot)}{/each}
    </ul>
  {/if}

  {#if teams.length}
    <h3 class="sb-head">{tr('Команды')}</h3>
    <ul class="k-list sb-list sb-tree" role="tree" aria-label={tr('Команды')} onkeydown={onTreeKey}>
      {#each teams as team (team.room.id)}
        {@render teamRow(team)}
        {#if team.lead}
          {@const open = isOpen(team.room.id)}
          <li class="sb-branch" class:is-open={open} role="none" {...(open ? {} : { inert: true })}>
            <ul class="sb-group" role="group">
              {@render botRow(team.lead.bot, { level: 2, parent: team.room.id, slot: `side:${team.lead.bot.id}:${team.room.id}`, repeat: team.lead.repeat })}
              {#if team.others.length}
                <li class="sb-branch is-open" role="none">
                  <ul class="sb-group sb-sub" role="group">
                    {#each team.others as other (other.bot.id)}
                      {@render botRow(other.bot, { level: 3, parent: team.room.id, slot: `side:${other.bot.id}:${team.room.id}`, repeat: other.repeat })}
                    {/each}
                  </ul>
                </li>
              {/if}
            </ul>
          </li>
        {/if}
      {/each}
    </ul>
  {/if}

  {#if main.length}
    {#if teams.length}
      <h3 class="sb-head" data-drop="loose">{tr('Без команды')}</h3>
    {:else if pinned.length}
      <h3 class="sb-head">{tr('Боты')}</h3>
    {/if}
    <ul class="k-list sb-list" data-drop={teams.length ? 'loose' : undefined}>
      {#each main as bot (bot.id)}{@render botRow(bot)}{/each}
    </ul>
  {/if}

  {#if reserve.length}
    <button type="button" class="sb-head sb-toggle" aria-expanded={reserveOpen} onclick={toggleReserve}>
      <Icon name="chevron-right" size={14} class={['sb-chev', reserveOpen && 'is-open']} />
      {tr('Специалисты в запасе')}
      <span class="sb-count">{reserve.length}</span>
    </button>
    {#if reserveOpen}
      <ul class="k-list sb-list">
        {#each reserve as bot (bot.id)}{@render botRow(bot)}{/each}
      </ul>
    {/if}
  {/if}

  {#if archivedRooms.length}
    <button
      type="button"
      class="sb-head sb-toggle"
      aria-expanded={archiveOpen}
      aria-label={tr('Архив команд · {n}', { n: archivedRooms.length })}
      onclick={toggleArchive}
    >
      <Icon name="chevron-right" size={14} class={['sb-chev', archiveOpen && 'is-open']} />
      {tr('Архив команд')}
      <span class="sb-count">{archivedRooms.length}</span>
    </button>
    {#if archiveOpen}
      <ul class="k-list sb-list">
        {#each archivedRooms as room (room.id)}{@render roomRow(room)}{/each}
      </ul>
    {/if}
  {/if}
{/if}

<style>
  .sb-list { margin: 0 0 var(--k-space-1); padding: 0; list-style: none; }
  .sb-head {
    display: flex;
    align-items: center;
    gap: var(--k-space-1);
    width: 100%;
    margin: var(--k-space-3) 0 var(--k-space-1);
    padding: 0 var(--k-space-3);
    border: 0;
    background: none;
    color: var(--k-text-3);
    font: 600 var(--k-text-xs) / 1.3 var(--k-font-body);
    text-align: left;
  }
  .sb-toggle { min-height: 28px; border-radius: var(--k-radius-sm); cursor: pointer; }
  .sb-toggle:hover { color: var(--k-text); background: var(--k-surface-2); }
  .sb-toggle:focus-visible { outline: none; box-shadow: var(--k-focus); }
  :global(.sb-chev) { transition: transform var(--k-dur-fast) var(--k-ease-out); }
  :global(.sb-chev.is-open) { transform: rotate(90deg); }
  .sb-count {
    margin-left: auto;
    padding: 1px 7px;
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font-variant-numeric: tabular-nums;
  }
  .sb-empty { margin: 0; padding: var(--k-space-5); color: var(--k-text-3); font: 400 var(--k-text-sm) / 1.4 var(--k-font-body); }
  .sb-avatar { width: 40px; height: 40px; }

  /* Дерево команд: ветка раскрывается по высоте (0fr → 1fr), линия слева ведёт глаз от команды к ботам. */
  .sb-branch {
    display: grid;
    grid-template-rows: 0fr;
    margin: 0;
    padding: 0;
    list-style: none;
    transition: grid-template-rows var(--k-dur-base) var(--k-ease-out);
  }
  .sb-branch.is-open { grid-template-rows: 1fr; }
  .sb-branch > .sb-group { min-height: 0; overflow: hidden; }
  .sb-group {
    margin: 0 0 0 19px;
    padding: 0;
    list-style: none;
    border-left: 1px solid var(--k-line);
  }
  .sb-sub { margin-left: 19px; }
  /* Свёрнутая ветка не держит фокус и не читается (inert), но и не мигает при раскрытии. */
  .sb-branch:not(.is-open) { visibility: hidden; transition: grid-template-rows var(--k-dur-base) var(--k-ease-out), visibility 0s linear var(--k-dur-base); }
  .sb-branch.is-open { visibility: visible; }
  .sb-teamlead { display: inline-flex; align-items: center; gap: 2px; }
  .sb-chev-btn {
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 24px;
    margin-left: -6px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-sm);
    background: transparent;
    color: var(--k-text-3);
    cursor: pointer;
  }
  .sb-chev-btn:hover { color: var(--k-text); background: var(--k-surface-2); }
  .sb-chev-btn:focus-visible { outline: none; box-shadow: var(--k-focus); }
  :global(.sb-row.sb-team .avatar.room-icon) { color: var(--team-color, var(--k-text-2)); }
  :global(.sb-row.is-repeat .k-list-item__title) { color: var(--k-text-2); font-style: italic; }
  .sb-members, .sb-roll {
    display: inline-grid;
    place-items: center;
    min-width: 20px;
    height: 20px;
    margin-left: 4px;
    padding: 0 6px;
    border-radius: var(--k-radius-pill);
    background: var(--k-surface-2);
    color: var(--k-text-2);
    font: 600 var(--k-text-xs) / 1 var(--k-font-body);
    font-variant-numeric: tabular-nums;
  }
  .sb-roll { background: var(--k-surface-3); color: var(--k-text); }
  .sb-roll.is-needs { background: var(--k-attention); color: var(--k-attention-text); }
  .sb-roll.is-error { background: var(--k-danger); color: #fff; }

  /* Булавка видна при наведении и у закреплённых; работает с клавиатуры. */
  .sb-pin {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill);
    background: transparent;
    color: var(--k-text-3);
    opacity: 0;
    cursor: pointer;
    transition: opacity var(--k-dur-fast) var(--k-ease-out), background var(--k-dur-fast) var(--k-ease-out);
  }
  :global(.sb-row:hover) .sb-pin,
  .sb-pin:focus-visible { opacity: 1; }
  .sb-pin:hover { background: var(--k-surface-3); color: var(--k-text); }
  .sb-pin:focus-visible { outline: none; box-shadow: var(--k-focus); }
  .sb-pin.is-on { opacity: 1; color: var(--k-brand-text); }
  .sb-gear[aria-expanded='true'] { opacity: 1; background: var(--k-surface-3); color: var(--k-text); }
  :global(body.side-collapsed) .sb-gear { display: none; }

  /* Узкая панель: только аватары, без линий и стрелок; закрытая команда остаётся закрытой. */
  :global(body.side-collapsed) .sb-group { margin-left: 0; border-left: 0; }
  :global(body.side-collapsed) .sb-chev-btn,
  :global(body.side-collapsed) .sb-members,
  :global(body.side-collapsed) .sb-roll { display: none; }

  @media (prefers-reduced-motion: reduce) {
    :global(.sb-chev), .sb-pin, .sb-branch { transition: none; }
  }
</style>
