<script>
  // Вкладка «Напоминания» окна «Рутины»: фраза с датой, опережение, каналы, проверка отправки.
  // Напоминание — рутина kind:"reminder": срок стрелкой таймера, модель не зовётся (src/reminders.rs).
  // Список приходит от окна (routines): оно уже хранит состояние и обновляет его по событиям.
  import { api as defaultApi, serverText } from '../../lib/api.js';
  import { tr } from '../../lib/i18n.js';
  import { planReminder } from '../../lib/reminders.js';
  import { channelsOf, flagsOf, isTelegramPaired } from '../../lib/channels.js';
  import Composer from './Composer.svelte';
  import ChannelChips from './ChannelChips.svelte';
import { Button, IconButton, Select, Switch, EmptyState, Icon, ListRow } from '../kit';

  let { api = defaultApi, routines = [], onchanged, onopentelegram } = $props();

  let composer = $state();
  let text = $state('');
  let lead = $state('0');
  let windows = $state(true);
  let telegram = $state(false);
  let telegramPaired = $state(false);
  let editingId = $state('');
  let parsed = $state(null);
  let submitError = $state('');
  let busy = $state(false);
  let tests = $state({}); // id -> { busy, results }

  const reminders = $derived(
    routines
      .filter((r) => r.kind === 'reminder')
      .slice()
      .sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.nextRun - b.nextRun),
  );
  const editing = $derived(reminders.find((r) => r.id === editingId) || null);

  const leadOptions = $derived([
    { value: '0', label: tr('В момент события') },
    { value: '5', label: tr('За 5 минут') },
    { value: '15', label: tr('За 15 минут') },
    { value: '60', label: tr('За час') },
  ]);

  const ERRORS = {
    when: () => tr('Напиши, когда напомнить: «завтра в 10», «в 18:30», «каждый день в 9»'),
    time: () => tr('Добавь время, например «в 9:00»'),
    past: () => tr('Это время уже прошло'),
    repeat: () => tr('Повтор пока только каждый день или по дням недели'),
  };

  // Подсказка под полем: что ещё не хватает фразе. В режиме правки пустой разбор — это «срок не менять».
  const hint = $derived.by(() => {
    if (!text.trim() || !parsed) return '';
    const plan = planReminder(parsed);
    if (!plan.error) return '';
    if (editing && !parsed.due_time && !parsed.due_date && !parsed.recurrence) return '';
    return ERRORS[plan.error]();
  });

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

  // Сработавшее напоминание меняет статус и срок: через миг просим окно обновить список. Опроса нет.
  $effect(() => {
    let timer = null;
    const onEvent = (e) => {
      if (e.detail?.type !== 'reminder') return;
      clearTimeout(timer);
      timer = setTimeout(() => onchanged?.(), 800);
    };
    window.addEventListener('deloist:event', onEvent);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('deloist:event', onEvent);
    };
  });

  function fail(err) {
    submitError = serverText(err?.data?.error || err?.message) || tr('Не удалось выполнить запрос');
  }

  function channelsNow() {
    return channelsOf({ windows, telegram, paired: telegramPaired });
  }

  function reset() {
    text = '';
    lead = '0';
    windows = true;
    telegram = false;
    editingId = '';
    submitError = '';
    parsed = null;
  }

  async function create(p) {
    submitError = '';
    const channels = channelsNow();
    if (!channels.length) {
      submitError = tr('Выбери хотя бы один способ доставки');
      return;
    }
    const hasWhen = Boolean(p.due_date || p.due_time || p.recurrence);
    let plan = null;
    if (!editing || hasWhen) {
      plan = planReminder(p);
      if (plan.error) {
        submitError = ERRORS[plan.error]();
        return;
      }
    }
    busy = true;
    try {
      if (editing) {
        const body = { leadMinutes: Number(lead), channels, prompt: (p.title || p.text).trim() };
        if (plan) {
          Object.assign(body, plan.fields, { enabled: true });
        }
        await api(`/api/routines/${encodeURIComponent(editing.id)}`, { method: 'PATCH', body });
      } else {
        await api('/api/routines', { method: 'POST', body: { kind: 'reminder', leadMinutes: Number(lead), channels, ...plan.fields } });
      }
      reset();
      await onchanged?.();
    } catch (err) {
      fail(err);
    } finally {
      busy = false;
    }
  }

  function startEdit(r) {
    editingId = r.id;
    text = r.prompt;
    lead = String(r.leadMinutes ?? 0);
    const flags = flagsOf(r.channels);
    windows = flags.windows;
    telegram = flags.telegram;
    submitError = '';
  }

  async function remove(r) {
    try {
      await api(`/api/routines/${encodeURIComponent(r.id)}`, { method: 'DELETE' });
      if (editingId === r.id) reset();
      await onchanged?.();
    } catch (err) {
      fail(err);
    }
  }

  async function toggleEnabled(r, on) {
    try {
      await api(`/api/routines/${encodeURIComponent(r.id)}`, { method: 'PATCH', body: { enabled: on } });
      await onchanged?.();
    } catch (err) {
      fail(err);
      await onchanged?.();
    }
  }

  export async function testSend(r) {
    tests[r.id] = { busy: true, results: null };
    try {
      const res = await api(`/api/routines/${encodeURIComponent(r.id)}/test-reminder`, { method: 'POST' });
      tests[r.id] = { busy: false, results: res?.deliveries ?? [] };
    } catch (err) {
      tests[r.id] = { busy: false, results: [{ channel: '', ok: false, error: serverText(err?.data?.error || err?.message) }] };
    }
  }

  const channelName = (c) => (c === 'telegram' ? tr('Telegram') : tr('Windows'));

  const locale = () => (typeof window !== 'undefined' && window.LOCALE) || undefined;
  function when(ms) {
    if (!ms) return '';
    const d = new Date(ms);
    const day = d.toLocaleDateString(locale(), { weekday: 'short', day: 'numeric', month: 'short' });
    const time = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
    return `${day}, ${time}`;
  }

  function repeatText(r) {
    const days = [...new Set((r.days || []).map(Number).filter((d) => d >= 1 && d <= 7))].sort((a, b) => a - b);
    const key = days.join('');
    const time = r.at;
    const names = [tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')];
    if (!days.length || days.length === 7) return tr('каждый день в {time}', { time });
    if (key === '12345') return tr('по будням в {time}', { time });
    if (key === '67') return tr('по выходным в {time}', { time });
    return tr('{days} в {time}', { days: days.map((d) => names[d - 1]).join(', '), time });
  }

  function scheduleText(r) {
    const eventAt = Number(r.onceAt) || 0;
    if (eventAt) return when(eventAt);
    return repeatText(r);
  }

  function nextText(r) {
    if (!r.enabled) return Number(r.onceAt) && r.lastRun ? tr('сработало') : tr('выключено');
    const at = Number(r.nextRun) + (Number(r.leadMinutes) || 0) * 60000;
    return tr('Следующее: {time}', { time: when(at) });
  }

  function statusText(r) {
    const map = {
      ok: tr('доставлено'),
      partial: tr('доставлено не везде'),
      error: tr('не доставлено'),
      missed: tr('пропущено: компьютер был выключен'),
      skipped: tr('пропущено'),
    };
    const label = map[r.lastStatus];
    return label ? { label, bad: r.lastStatus !== 'ok', at: when(r.lastRun) } : null;
  }

  function leadText(r) {
    const m = Number(r.leadMinutes) || 0;
    if (!m) return '';
    return m === 60 ? tr('за час') : tr('за {minutes} мин', { minutes: m });
  }

  function testText(t) {
    if (!t.results) return '';
    if (!t.results.length) return tr('Нет способов доставки');
    return t.results
      .map((d) => (d.ok ? tr('{channel}: отправлено', { channel: channelName(d.channel) }) : tr('{channel}: не вышло ({reason})', { channel: d.channel ? channelName(d.channel) : tr('Проверка'), reason: serverText(d.error) || '?' })))
      .join('; ');
  }
</script>

<div class="rm">
  <div class="rm-form">
    <div class="rm-form-head">
      <h4 class="rm-h">{editing ? tr('Изменить напоминание') : tr('Новое напоминание')}</h4>
      {#if editing}<Button class="rm-link" variant="ghost" size="sm" onclick={reset}>{tr('Отмена')}</Button>{/if}
    </div>
    <Composer
      bind:this={composer}
      bind:value={text}
      {api}
      placeholder={tr('Позвонить маме завтра в 18:00')}
      onsubmit={create}
      onparse={(p) => (parsed = p)}
    />
    {#if editing}
      <p class="rm-hint">{tr('Сейчас: {when}. Напиши новую дату, чтобы поменять.', { when: scheduleText(editing) })}</p>
    {/if}
    {#if hint && !submitError}<p class="rm-hint">{hint}</p>{/if}
    {#if submitError}<p class="rm-error" role="alert">{submitError}</p>{/if}

    <div class="rm-opts">
      <label class="rm-opt">
        <span class="rm-label">{tr('Когда напомнить')}</span>
        <Select options={leadOptions} bind:value={lead} aria-label={tr('Когда напомнить')} />
      </label>
      <div class="rm-opt">
        <span class="rm-label">{tr('Куда прислать')}</span>
        <ChannelChips bind:windows bind:telegram paired={telegramPaired} {onopentelegram} />
      </div>
    </div>

    <div class="rm-actions">
      <Button variant="primary" loading={busy} disabled={!text.trim()} onclick={() => composer?.submit()}>{editing ? tr('Сохранить') : tr('Добавить')}</Button>
    </div>
  </div>

  {#if !reminders.length}
    <EmptyState title={tr('Напоминаний пока нет')} text={tr('Напиши, о чём и когда напомнить. Код сам найдёт дату и время, модель не тратится.')} />
  {:else}
    <ul class="k-lrows">
      {#each reminders as r (r.id)}
        {@const status = statusText(r)}
        {@const test = tests[r.id]}
        <ListRow class="rm-item {r.enabled ? '' : 'is-off'}" title={r.prompt} titleClass="rm-title">
          {#snippet meta()}
            <span class="rm-meta">
              <span>{scheduleText(r)}</span>
              {#if leadText(r)}<span>{leadText(r)}</span>{/if}
              <span>{(r.channels?.length ? r.channels : ['windows']).map(channelName).join(', ')}</span>
            </span>
            <span class="rm-meta">
              <span>{nextText(r)}</span>
              {#if status}<span class:rm-bad={status.bad}>{status.label}{status.at ? `, ${status.at}` : ''}</span>{/if}
            </span>
            {#if test?.results}<span class="rm-test" class:rm-bad={test.results.some((d) => !d.ok)} role="status">{testText(test)}</span>{/if}
          {/snippet}
          {#snippet trailing()}
            <span class="rm-side">
              <Button size="sm" variant="ghost" loading={test?.busy} onclick={() => testSend(r)}>{tr('Проверить')}</Button>
              <Switch checked={r.enabled} label={tr('Включено')} onchange={(on) => toggleEnabled(r, on)} />
              <IconButton label={tr('Изменить')} size="sm" onclick={() => startEdit(r)}>
                {#snippet icon()}
                  <Icon name="pencil" size={16} />
                {/snippet}
              </IconButton>
              <IconButton label={tr('Удалить')} size="sm" onclick={() => remove(r)}>
                {#snippet icon()}
                  <Icon name="close" size={16} />
                {/snippet}
              </IconButton>
            </span>
          {/snippet}
        </ListRow>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .rm {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-4, 16px);
    padding-top: var(--k-space-2, 8px);
  }
  .rm-form {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3, 12px);
    padding: var(--k-space-4, 16px);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg, 26px);
    background: var(--k-surface-2);
  }
  .rm-form-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .rm-h {
    margin: 0;
    color: var(--k-text);
    font: 700 var(--k-text-md, 15px) / 1.3 var(--k-font-body, system-ui);
  }
  .rm-hint,
  .rm-error {
    margin: 0;
    font: 400 var(--k-text-sm, 13px) / 1.45 var(--k-font-body, system-ui);
  }
  .rm-hint {
    color: var(--k-text-3);
  }
  .rm-error {
    color: var(--k-danger-text);
    font-weight: 600;
  }
  .rm-opts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-4, 16px);
  }
  .rm-opt {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 160px;
  }
  .rm-label {
    color: var(--k-text-2);
    font: 600 var(--k-text-xs, 12px) / 1.3 var(--k-font-body, system-ui);
    letter-spacing: 0.04em;
  }
  .rm-actions {
    display: flex;
    justify-content: flex-end;
  }
  :global(.rm-item.is-off .k-lrow__title) {
    color: var(--k-text-2);
  }
  .rm-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 2px var(--k-space-3, 12px);
    color: var(--k-text-3);
    font: 400 var(--k-text-xs, 12px) / 1.5 var(--k-font-body, system-ui);
  }
  .rm-test {
    margin-top: 4px;
    color: var(--k-text-2);
    font: 600 var(--k-text-xs, 12px) / 1.4 var(--k-font-body, system-ui);
  }
  .rm-bad {
    color: var(--k-danger-text);
  }
  .rm-side {
    display: flex;
    align-items: center;
    gap: var(--k-space-2, 8px);
  }
</style>
