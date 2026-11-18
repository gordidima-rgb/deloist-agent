<script>
  // Экран «Сторож страниц» окна настроек: рутина по расписанию сама открывает страницы в браузере бота
  // и будит модель, только если на них появились новые записи. Без новых модель не вызывается.
  import { untrack } from 'svelte';
  import Section from '../kit/Section.svelte';
  import Field from '../kit/Field.svelte';
  import TextInput from '../kit/TextInput.svelte';
  import TextArea from '../kit/TextArea.svelte';
  import Select from '../kit/Select.svelte';
  import Button from '../kit/Button.svelte';
  import { toast } from '../../lib/toast.js';
  import { tr } from '../../lib/i18n.js';

  let { api, routines = [], onchanged } = $props();

  let data = $state(null);
  let loading = $state(true);
  let error = $state('');

  let routineId = $state('');
  let urlsText = $state('');
  let selector = $state('article');
  let contains = $state('');
  let saving = $state(false);
  let formError = $state('');

  const triggers = $derived(data?.triggers ?? []);
  const eligible = $derived(routines.filter((r) => !r.hook && r.kind !== 'reminder'));
  const routineOptions = $derived(eligible.map((r) => ({ value: r.id, label: r.title })));
  const active = $derived(
    triggers
      .map((t) => ({ ...t, routine: routines.find((r) => r.id === t.routineId) || null }))
      .filter((t) => t.routine),
  );

  $effect(() => {
    if (eligible.length && !eligible.some((r) => r.id === routineId)) routineId = eligible[0].id;
  });

  const errText = (err) => err?.message || tr('Не удалось выполнить запрос');
  const urlList = (text) => [...new Set(String(text || '').split(/\s+/).filter(Boolean))];

  async function load() {
    try {
      data = await api('/api/watch');
      error = '';
    } catch (err) {
      error = errText(err);
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (saving || !routineId) return;
    saving = true;
    formError = '';
    try {
      await api(`/api/watch/triggers/${encodeURIComponent(routineId)}`, {
        method: 'PUT',
        body: { urls: urlList(urlsText), selector: selector.trim(), contains: contains.trim() },
      });
      urlsText = '';
      contains = '';
      toast(tr('Сторож сохранён'));
      await load();
      onchanged?.();
    } catch (err) {
      formError = errText(err);
    } finally {
      saving = false;
    }
  }

  async function remove(id) {
    try {
      await api(`/api/watch/triggers/${encodeURIComponent(id)}`, { method: 'DELETE' });
      await load();
      onchanged?.();
    } catch (err) {
      toast(errText(err));
    }
  }

  function when(sec) {
    const n = Number(sec);
    if (!Number.isFinite(n) || n <= 0) return tr('ещё не проверял');
    return new Date(n).toLocaleString();
  }

  $effect(() => {
    untrack(() => {
      load();
    });
  });
</script>

{#if loading && !data}
  <Section title={tr('Сторож страниц')}>
    <p class="wt-note">{tr('Загружаю…')}</p>
  </Section>
{:else if !data}
  <Section title={tr('Сторож страниц')}>
    <p class="k-hint is-error" role="alert">{error}</p>
    <div class="wt-actions"><Button onclick={load}>{tr('Повторить')}</Button></div>
  </Section>
{:else}
  <Section
    title={tr('Новые записи будят рутину')}
    description={tr('Рутина по своему расписанию открывает страницы в браузере бота и будит модель, только если появились новые подходящие записи. Без новых модель не вызывается. Первая проверка только запоминает записи, каждая обрабатывается один раз. У рутины один триггер: почта или страницы.')}
  >
    {#each active as t (t.routineId)}
      <div class="wt-card" data-watch={t.routineId}>
        <span class="wt-lead">{t.routine.title}</span>
        <span class="wt-note">{t.urls.join(' · ')}</span>
        <span class="wt-note">
          {tr('селектор: {value}', { value: t.selector })}{t.contains ? ' · ' + tr('содержит: {value}', { value: t.contains }) : ''}
        </span>
        <span class="wt-note">{tr('последняя проверка: {value}', { value: when(t.lastCheck) })}</span>
        {#if t.lastError}<p class="k-hint is-error">{t.lastError}</p>{/if}
        <div class="wt-actions">
          <Button variant="attention" onclick={() => remove(t.routineId)}>{tr('Убрать')}</Button>
        </div>
      </div>
    {/each}
    {#if !eligible.length}
      <p class="wt-note">{tr('Нет рутин, которые можно привязать к страницам. Создайте рутину по расписанию.')}</p>
    {:else}
      <Field label={tr('Рутина')}>
        {#snippet children(f)}
          <Select id={f.id} options={routineOptions} bind:value={routineId} />
        {/snippet}
      </Field>
      <Field label={tr('Страницы')} hint={tr('По одному адресу в строке, до 10. Например, https://x.com/имя или https://x.com/notifications.')}>
        {#snippet children(f)}
          <TextArea id={f.id} rows={3} spellcheck="false" bind:value={urlsText} />
        {/snippet}
      </Field>
      <Field label={tr('Селектор записи')} hint={tr('CSS-селектор одной записи. Для X подойдёт article.')}>
        {#snippet children(f)}
          <TextInput id={f.id} spellcheck="false" bind:value={selector} />
        {/snippet}
      </Field>
      <Field label={tr('Текст записи содержит')} hint={tr('Без учёта регистра. Пусто — любая запись.')}>
        {#snippet children(f)}
          <TextInput id={f.id} bind:value={contains} />
        {/snippet}
      </Field>
      {#if formError}<p class="k-hint is-error" role="alert">{formError}</p>{/if}
      <div class="wt-actions">
        <Button variant="primary" loading={saving} disabled={!urlList(urlsText).length} onclick={save}>
          {tr('Сохранить сторожа')}
        </Button>
      </div>
    {/if}
  </Section>
{/if}

<style>
  :global(#sub-watch .k-section__body) {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-3);
    padding-top: var(--k-space-4);
    padding-bottom: var(--k-space-5);
  }
  .wt-lead {
    color: var(--k-text);
    font: 600 var(--k-text-md) / 1.45 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .wt-note {
    margin: 0;
    color: var(--k-text-3);
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    overflow-wrap: anywhere;
  }
  .wt-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--k-space-2);
  }
  .wt-card {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    padding: var(--k-space-4);
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-lg);
    background: var(--k-surface-2);
  }
</style>
