<script>
  // Вкладка «Привычки» окна «Рутины»: отметка за сегодня, серия, неделя, добавление, правка и галерея шаблонов.
  // Данные берёт сама у /api/habits и обновляется по событию habits_changed (его шлёт окно через deloist:event).
  import { api as defaultApi, lang, serverText } from '../../lib/api.js';
  import { tr } from '../../lib/i18n.js';
  import { HABIT_ICON_NAMES } from '../../lib/habitIcons.js';
  import { daysFromMask, maskFromDays } from '../../lib/habits.js';
  import { channelsOf, flagsOf, isTelegramPaired } from '../../lib/channels.js';
import { Button, IconButton, Field, TextInput, Chip, EmptyState, Icon } from '../kit';
  import HabitIcon from './HabitIcon.svelte';
  import ChannelChips from './ChannelChips.svelte';

  let { api = defaultApi, onopentelegram } = $props();

  let habits = $state([]);
  let loaded = $state(false);
  let error = $state('');
  let templates = $state(null);
  let gallery = $state(false);
  // { id, title, icon, days: boolean[7], remindAt: 'ЧЧ:ММ' | '' (пусто — без напоминания), windows, telegram, confirmArchive }
  let form = $state(null);
  let saving = $state(false);
  let telegramPaired = $state(false);
  const pending = new Set();

  const dayNames = $derived([tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')]);
  const todayList = $derived(habits.filter((h) => h.today_scheduled));
  const doneToday = $derived(todayList.filter((h) => h.today_done).length);

  // Telegram доступен, только когда чат привязан: статус спрашиваем один раз при открытии вкладки.
  $effect(() => {
    let alive = true;
    isTelegramPaired(api).then((paired) => {
      if (alive) telegramPaired = paired;
    });
    return () => {
      alive = false;
    };
  });

  function fail(err) {
    error = serverText(err?.data?.message || err?.message) || tr('Не удалось выполнить запрос');
  }

  let loading = false;
  let again = false;
  async function load() {
    if (loading) {
      again = true;
      return;
    }
    loading = true;
    try {
      const list = await api('/api/habits');
      if (Array.isArray(list)) habits = list;
      error = '';
    } catch (err) {
      fail(err);
    } finally {
      loading = false;
      loaded = true;
      if (again) {
        again = false;
        load();
      }
    }
  }

  $effect(() => {
    load();
    const onEvent = (e) => {
      if (e.detail?.type === 'habits_changed') load();
    };
    window.addEventListener('deloist:event', onEvent);
    return () => window.removeEventListener('deloist:event', onEvent);
  });

  function put(next) {
    habits = habits.map((h) => (h.id === next.id ? next : h));
  }

  // Отметка сразу меняет вид, ответ сервера её подтверждает; при отказе возвращаем как было.
  export async function toggle(habit) {
    if (pending.has(habit.id)) return;
    pending.add(habit.id);
    const before = habit;
    const on = !habit.today_done;
    put({
      ...habit,
      today_done: on,
      week_done: Math.max(0, habit.week_done + (habit.today_scheduled ? (on ? 1 : -1) : 0)),
      streak: Math.max(0, habit.streak + (on ? 1 : -1)),
    });
    try {
      const next = on
        ? await api(`/api/habits/${encodeURIComponent(habit.id)}/check`, { method: 'POST' })
        : await api(`/api/habits/${encodeURIComponent(habit.id)}/check`, { method: 'DELETE' });
      error = '';
      if (next?.id) put(next);
    } catch (err) {
      put(before);
      fail(err);
    } finally {
      pending.delete(habit.id);
    }
  }

  function openForm(habit) {
    gallery = false;
    form = habit
      ? { id: habit.id, title: habit.title, icon: habit.icon, days: daysFromMask(habit.days_mask), remindAt: habit.remind_at ?? '', ...flagsOf(habit.remind_channels), confirmArchive: false }
      : { id: '', title: '', icon: 'check', days: daysFromMask(127), remindAt: '', windows: true, telegram: false, confirmArchive: false };
  }

  const formMask = $derived(form ? maskFromDays(form.days) : 0);
  const remindChannels = $derived(form ? channelsOf({ windows: form.windows, telegram: form.telegram, paired: telegramPaired }) : []);
  // Напоминание без единого канала не сохраняем: сервер такой список отвергнет.
  const remindNeedsChannel = $derived(Boolean(form?.remindAt) && !remindChannels.length);
  const canSave = $derived(Boolean(form && form.title.trim() && formMask > 0 && !saving && !remindNeedsChannel));

  async function save() {
    if (!form || !canSave) return;
    saving = true;
    try {
      const body = { title: form.title.trim(), icon: form.icon, days_mask: formMask, remind_at: form.remindAt };
      if (form.remindAt) body.remind_channels = remindChannels;
      if (form.id) {
        const next = await api(`/api/habits/${encodeURIComponent(form.id)}`, { method: 'PATCH', body });
        if (next?.id) put(next);
      } else {
        const next = await api('/api/habits', { method: 'POST', body });
        if (next?.id) habits = [...habits, next];
      }
      error = '';
      form = null;
    } catch (err) {
      fail(err);
    } finally {
      saving = false;
    }
  }

  async function archive() {
    if (!form?.id) return;
    if (!form.confirmArchive) {
      form.confirmArchive = true;
      return;
    }
    saving = true;
    try {
      await api(`/api/habits/${encodeURIComponent(form.id)}`, { method: 'DELETE' });
      habits = habits.filter((h) => h.id !== form.id);
      error = '';
      form = null;
    } catch (err) {
      fail(err);
    } finally {
      saving = false;
    }
  }

  async function openGallery() {
    form = null;
    gallery = !gallery;
    if (gallery && !templates) {
      try {
        templates = await api('/api/habits/templates');
      } catch (err) {
        fail(err);
        gallery = false;
      }
    }
  }

  const templateTitle = (t) => (lang() === 'en' ? t.en || t.title : t.title);
  const groupTitle = (g) => (lang() === 'en' ? g.en || g.title : g.title);
  const taken = (t) => habits.some((h) => h.title === templateTitle(t));

  async function addTemplate(t) {
    if (taken(t) || saving) return;
    saving = true;
    try {
      const next = await api('/api/habits', { method: 'POST', body: { title: templateTitle(t), icon: t.icon, days_mask: t.days_mask } });
      if (next?.id) habits = [...habits, next];
      error = '';
    } catch (err) {
      fail(err);
    } finally {
      saving = false;
    }
  }

  function weekText(h) {
    return tr('{done} из {total} за неделю', { done: h.week_done, total: h.week_scheduled });
  }
</script>

<div class="hb">
  {#if error}<p class="hb-error" role="alert">{error}</p>{/if}

  <div class="hb-bar">
    <p class="hb-sum">
      {#if loaded && todayList.length}{tr('Сегодня: {done} из {total}', { done: doneToday, total: todayList.length })}{:else}{tr('Маленькие дела, которые хочется делать каждый день.')}{/if}
    </p>
    <div class="hb-bar-actions">
      <Button size="sm" variant="ghost" onclick={openGallery} aria-expanded={gallery}>{tr('Шаблоны')}</Button>
      <Button size="sm" variant="primary" onclick={() => openForm(null)}>{tr('Новая привычка')}</Button>
    </div>
  </div>

  {#if form}
    <form class="hb-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
      <Field label={tr('Название')}>
        {#snippet children({ id, describedby, invalid })}
          <TextInput {id} {describedby} {invalid} bind:value={form.title} maxlength="80" placeholder={tr('Выпить стакан воды')} />
        {/snippet}
      </Field>
      <div class="hb-field">
        <span class="hb-label">{tr('Значок')}</span>
        <div class="hb-icons" role="radiogroup" aria-label={tr('Значок')}>
          {#each HABIT_ICON_NAMES as name (name)}
            <button type="button" class="hb-ico" role="radio" aria-checked={form.icon === name} aria-label={name} onclick={() => (form.icon = name)}>
              <HabitIcon {name} size={20} />
            </button>
          {/each}
        </div>
      </div>
      <div class="hb-field">
        <span class="hb-label">{tr('Дни недели')}</span>
        <div class="hb-days">
          {#each dayNames as label, i (i)}
            <Chip bind:selected={form.days[i]}>{label}</Chip>
          {/each}
        </div>
      </div>
      <div class="hb-field">
        <span class="hb-label">{tr('Напомнить')}</span>
        <div class="hb-remind">
          <input type="time" class="hb-time" aria-label={tr('Время напоминания')} bind:value={form.remindAt} />
          {#if form.remindAt}
            <button type="button" class="hb-link" onclick={() => (form.remindAt = '')}>{tr('Не напоминать')}</button>
          {:else}
            <span class="hb-hint">{tr('Пусто — без напоминания.')}</span>
          {/if}
        </div>
        {#if form.remindAt}
          <ChannelChips bind:windows={form.windows} bind:telegram={form.telegram} paired={telegramPaired} {onopentelegram} />
          {#if remindNeedsChannel}<p class="hb-hint">{tr('Выбери, куда прислать напоминание.')}</p>{/if}
        {/if}
      </div>
      <div class="hb-form-actions">
        <Button variant="primary" type="submit" disabled={!canSave} loading={saving}>{form.id ? tr('Сохранить') : tr('Добавить')}</Button>
        <Button variant="ghost" onclick={() => (form = null)}>{tr('Отмена')}</Button>
        {#if form.id}
          <span class="hb-spacer"></span>
          <Button variant={form.confirmArchive ? 'attention' : 'ghost'} onclick={archive}>{form.confirmArchive ? tr('Точно убрать?') : tr('В архив')}</Button>
        {/if}
      </div>
    </form>
  {/if}

  {#if gallery && templates}
    <div class="hb-gallery">
      {#each templates.groups as group (group.id)}
        <div class="hb-group">
          <h5 class="hb-group-title">{groupTitle(group)}</h5>
          <div class="hb-tpls">
            {#each templates.templates.filter((t) => t.group === group.id) as tpl (tpl.title)}
              <button type="button" class="hb-tpl" disabled={taken(tpl) || saving} onclick={() => addTemplate(tpl)}>
                <HabitIcon name={tpl.icon} size={18} />
                <span>{templateTitle(tpl)}</span>
                {#if taken(tpl)}<span class="hb-tpl-mark">{tr('добавлена')}</span>{/if}
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}

  {#if loaded && !habits.length && !form && !gallery}
    <EmptyState title={tr('Привычек пока нет')} text={tr('Начни с шаблона или заведи свою: отметка за день занимает одну секунду.')}>
      {#snippet action()}
        <Button variant="primary" onclick={openGallery}>{tr('Выбрать из шаблонов')}</Button>
      {/snippet}
    </EmptyState>
  {/if}

  {#if habits.length}
    <ul class="hb-list">
      {#each habits as h (h.id)}
        <li class="hb-item" class:hb-item--done={h.today_done} class:hb-item--off={!h.today_scheduled}>
          <button
            type="button"
            class="hb-check"
            class:hb-check--on={h.today_done}
            role="checkbox"
            aria-checked={h.today_done}
            aria-label={tr('Отметить: {title}', { title: h.title })}
            onclick={() => toggle(h)}
          >
            <Icon name="check" size={22} stroke={2.6} />
          </button>
          <span class="hb-icon"><HabitIcon name={h.icon} size={20} /></span>
          <div class="hb-main">
            <div class="hb-title">{h.title}</div>
            <div class="hb-meta">
              <span>{tr('Серия: {n} дн.', { n: h.streak })}</span>
              <span>{weekText(h)}</span>
              {#if h.remind_at}
                <span class="hb-bell" title={tr('Напоминание в {time}', { time: h.remind_at })}>
                  <Icon name="bell-small" size={12} stroke={2.2} />
                  {h.remind_at}
                </span>
              {/if}
              {#if !h.today_scheduled}<span>{tr('сегодня не по расписанию')}</span>{/if}
            </div>
          </div>
          <div class="hb-week" aria-hidden="true">
            {#each h.week as d (d.day)}
              <span class="hb-dot" class:hb-dot--on={d.done} class:hb-dot--skip={!d.scheduled} title={d.day}></span>
            {/each}
          </div>
          <IconButton label={tr('Изменить')} size="sm" onclick={() => openForm(h)}>
            {#snippet icon()}
              <Icon name="pencil" size={16} />
            {/snippet}
          </IconButton>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .hb {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-4, 16px);
    padding-top: var(--k-space-2, 8px);
  }
  .hb-error {
    margin: 0;
    padding: var(--k-space-2, 8px) var(--k-space-3, 12px);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-attention-tint, #ffe8e2);
    color: var(--k-attention-text, #b4321b);
    font: 500 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
  }
  .hb-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3, 12px);
  }
  .hb-sum {
    margin: 0;
    color: var(--k-text-2, #5c5f66);
    font: 600 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
  }
  .hb-bar-actions {
    display: flex;
    gap: var(--k-space-2, 8px);
  }
  .hb-form,
  .hb-gallery {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-4, 16px);
    padding: var(--k-space-4, 16px);
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2, #f1f0ec);
  }
  .hb-field {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .hb-label {
    color: var(--k-text-2, #5c5f66);
    font: 600 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, system-ui);
    letter-spacing: 0.04em;
  }
  .hb-icons,
  .hb-days,
  .hb-tpls {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2, 8px);
  }
  .hb-ico {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    padding: 0;
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface, #fff);
    color: var(--k-text-2, #5c5f66);
    cursor: pointer;
    transition: background 120ms var(--k-ease-out, ease), color 120ms var(--k-ease-out, ease);
  }
  .hb-ico:hover {
    color: var(--k-text, #262931);
  }
  .hb-ico[aria-checked='true'] {
    border-color: var(--k-brand, #025bff);
    background: var(--k-brand-tint, #e7efff);
    color: var(--k-brand-text, #0250e0);
  }
  .hb-ico:focus-visible,
  .hb-check:focus-visible,
  .hb-tpl:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  .hb-remind {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-3, 12px);
  }
  .hb-time {
    min-height: 38px;
    padding: 0 var(--k-space-3, 12px);
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface, #fff);
    color: var(--k-text, #262931);
    font: 500 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
  }
  .hb-time:focus-visible {
    outline: none;
    box-shadow: var(--k-focus, 0 0 0 3px rgba(2, 91, 255, 0.28));
  }
  .hb-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--k-brand-text, #0250e0);
    font: 600 var(--k-text-sm, 13px) / 1.4 var(--k-font-body, system-ui);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .hb-hint {
    margin: 0;
    color: var(--k-text-3, #8d8f94);
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .hb-bell {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--k-brand-text, #0250e0);
  }
  .hb-form-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--k-space-2, 8px);
  }
  .hb-spacer {
    flex: 1;
  }
  .hb-group {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
  }
  .hb-group-title {
    margin: 0;
    color: var(--k-text-3, #8d8f94);
    font: 700 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, system-ui);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .hb-tpl {
    display: inline-flex;
    align-items: center;
    gap: var(--k-space-2, 8px);
    padding: 8px 14px 8px 10px;
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-surface, #fff);
    color: var(--k-text, #262931);
    font: 500 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, system-ui);
    cursor: pointer;
  }
  .hb-tpl:hover:not(:disabled) {
    border-color: var(--k-brand, #025bff);
  }
  .hb-tpl:disabled {
    cursor: default;
    opacity: 0.55;
  }
  .hb-tpl-mark {
    color: var(--k-text-3, #8d8f94);
    font-size: var(--k-text-xs, 12px);
  }
  .hb-list {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2, 8px);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .hb-item {
    display: flex;
    align-items: center;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-3, 12px) var(--k-space-4, 16px);
    border: 1px solid var(--k-line, #e0ded9);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface, #fff);
  }
  .hb-item--off .hb-title {
    color: var(--k-text-2, #5c5f66);
  }
  .hb-check {
    display: grid;
    flex: none;
    place-items: center;
    width: 44px;
    height: 44px;
    padding: 0;
    border: 2px solid var(--k-line-strong, #cfcdc7);
    border-radius: var(--k-radius-pill, 999px);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: background 160ms var(--k-ease-out, ease), border-color 160ms var(--k-ease-out, ease), color 160ms var(--k-ease-out, ease), transform 160ms var(--k-ease-out, ease);
  }
  .hb-check:hover {
    border-color: var(--k-brand, #025bff);
  }
  .hb-check:active {
    transform: scale(0.94);
  }
  .hb-check--on {
    border-color: var(--k-brand, #025bff);
    background: var(--k-brand, #025bff);
    color: var(--k-on-brand, #fff);
  }
  .hb-icon {
    display: grid;
    flex: none;
    place-items: center;
    width: 32px;
    color: var(--k-text-2, #5c5f66);
  }
  .hb-main {
    flex: 1;
    min-width: 0;
  }
  .hb-title {
    overflow: hidden;
    color: var(--k-text, #262931);
    font: 600 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hb-item--done .hb-title {
    color: var(--k-text-2, #5c5f66);
  }
  .hb-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--k-space-3, 12px);
    color: var(--k-text-3, #8d8f94);
    font: 400 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, system-ui);
  }
  .hb-week {
    display: flex;
    flex: none;
    gap: 4px;
  }
  .hb-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--k-radius-pill, 999px);
    background: var(--k-line-strong, #cfcdc7);
  }
  .hb-dot--on {
    background: var(--k-brand, #025bff);
  }
  .hb-dot--skip {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--k-line, #e0ded9);
  }
  @media (max-width: 520px) {
    .hb-week {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .hb-check,
    .hb-ico {
      transition: none;
    }
    .hb-check:active {
      transform: none;
    }
  }
</style>
