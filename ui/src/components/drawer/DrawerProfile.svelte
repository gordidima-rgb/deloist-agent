<script>
  // Вкладка «Профиль» в ящике бота: разделы кита вместо одной длинной формы.
  // Раскладка по образцу OpenBot: внешность, кто это, модель, разрешения, навыки, опасное внизу.
  // Конструктор внешности и выбор модели монтирует app.js в свои места (mountLook, mountModel):
  // у них свои движки аватаров и список провайдеров. Сохранение собирает app.js (onsave).
  import { untrack } from 'svelte';
  import Section from '../kit/Section.svelte';
  import Field from '../kit/Field.svelte';
  import TextInput from '../kit/TextInput.svelte';
  import TextArea from '../kit/TextArea.svelte';
  import Select from '../kit/Select.svelte';
  import Slider from '../kit/Slider.svelte';
  import Switch from '../kit/Switch.svelte';
  import Checkbox from '../kit/Checkbox.svelte';
  import Button from '../kit/Button.svelte';
  import { tr } from '../../lib/i18n.js';

  let {
    bot,
    skills = [],
    levels = {},
    petBlocked = false,
    petOn = undefined,
    folder = '',
    mountLook,
    mountModel,
    contextInfo,
    onsave,
    ondelete,
    onpet,
    ongoogle,
    onrevokesite,
  } = $props();

  const start = untrack(() => bot);
  let name = $state(start.name || '');
  let role = $state(start.role || '');
  let persona = $state(start.persona || '');
  let driver = $state(start.driver || '');
  let model = $state(start.model || '');
  let level = $state(start.modelLevel || '');
  let effort = $state(start.effort || 'medium');
  let approval = $state(start.approval || 'ask');
  // Порог сжатия в процентах окна. Пока ползунок не тронут и у бота нет своего значения, работает прежний режим
  // (`contextMode`): сохранение профиля его не меняет и ничего не удорожает молча.
  const COMPACT_DEFAULT = 60;
  const COMPACT_MARKS = [50, 80];
  const storedPercent = Number(start.compactAtPercent);
  let percent = $state(start.compactAtPercent != null && Number.isFinite(storedPercent) ? storedPercent : COMPACT_DEFAULT);
  let touched = $state(false);
  // Месячный предел токенов: пусто — без предела (0 на сервере).
  let monthLimit = $state(start.monthlyTokenLimit > 0 ? String(start.monthlyTokenLimit) : '');
  let pet = $state(Boolean(start.pet));
  let chosen = $state(Object.fromEntries(untrack(() => skills).map((s) => [s.id, (start.skills || []).includes(s.id)])));
  let info = $state(null);
  let saving = $state(false);

  // app.js сообщает, что сохранилось на самом деле (стол мог быть полон или запрос упал).
  $effect(() => {
    if (petOn !== undefined) pet = petOn;
  });

  const levelOptions = $derived([
    { value: '', label: tr('Авто (как выбрано)') },
    { value: 'strong', label: tr('Сильная') },
    { value: 'standard', label: tr('Обычная') },
    { value: 'fast', label: tr('Быстрая') },
  ]);
  const effortOptions = $derived([
    { value: 'medium', label: tr('Обычное') },
    { value: 'high', label: tr('Высокое') },
    { value: 'low', label: tr('Низкое') },
  ]);
  const approvalOptions = $derived([
    { value: 'ask', label: tr('на изменения вне своей папки') },
    { value: 'strict', label: tr('на всё, кроме чтения') },
    { value: 'auto', label: tr('не спрашивать') },
  ]);
  // Если порог не задан, ползунок стоит там, где его держит старый режим (по ответу сервера).
  $effect(() => {
    if (!touched && info && !info.stored && Number.isFinite(Number(info.percent))) percent = Number(info.percent);
  });
  const sliderMin = $derived(info?.min ?? 20);
  const sliderMax = $derived(info?.max ?? 95);
  const sliderStep = $derived(info?.step ?? 5);
  // Окно считает сервер (одна таблица моделей); здесь только доля от него и потолок с запасом под ответ.
  const windowTokens = $derived(info?.window ?? 40000);
  const usable = $derived(info?.usable ?? Math.floor(windowTokens * 0.8));
  const rawTokens = $derived(Math.floor((windowTokens * percent) / 100));
  const limitTokens = $derived(Math.min(rawTokens, usable));
  const isDefault = $derived(Boolean(!touched && info && !info.stored));
  const shownTokens = $derived(isDefault ? info.currentTokens : limitTokens);
  const monthNote = $derived.by(() => {
    const used = fmt(bot.tokensMonth || 0);
    const cap = Number(String(monthLimit).replace(/[\s_]/g, '')) || 0;
    return cap > 0
      ? tr('В этом месяце: {used} из {cap} токенов. Когда лимит кончится, бот не ответит до следующего месяца или пока ты его не поднимешь.', { used, cap: fmt(cap) })
      : tr('В этом месяце: {used} токенов. Пусто — без лимита.', { used });
  });
  const tokensLine = $derived(
    info?.known
      ? tr('≈ {n} из {total} токенов', { n: fmt(shownTokens), total: fmt(windowTokens) })
      : tr('Размер окна этой модели неизвестен, считаю от {total} токенов: ≈ {n}. Агенты ACP и программы ведут свой контекст сами.', {
          total: fmt(windowTokens),
          n: fmt(shownTokens),
        }),
  );
  const costWarn = $derived(shownTokens > 200000);
  const capped = $derived(!isDefault && rawTokens > usable);
  function moved() {
    touched = true;
  }
  // «сейчас: …»: ручная модель, иначе модель выбранного уровня, иначе модель из настроек.
  const levelNote = $derived.by(() => {
    let now = model.trim();
    if (!now && level) {
      const provider = driver.startsWith('openai:') ? driver.slice(7) : driver;
      now = levels?.[provider]?.[level] || '';
    }
    return now ? tr('сейчас: {model}', { model: now }) : tr('сейчас: как в настройках');
  });

  function fmt(n) {
    return Number(n || 0).toLocaleString('ru-RU');
  }

  // Размер «памяти разговора» зависит от модели: спрашиваем заново, когда она меняется.
  $effect(() => {
    const query = { driver, model: model.trim() };
    let alive = true;
    Promise.resolve(contextInfo?.(query))
      .then((next) => {
        if (alive) info = next || null;
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  });

  // Выбрали уровень — модель подбирает программа, ручной выбор уступает (и наоборот).
  let modelHandle = null;
  function pickLevel(next) {
    level = next;
    if (next && model) {
      model = '';
      modelHandle?.set({ driver, model: '' });
    }
  }

  const lookHost = (el) => mountLook?.(el);
  const modelHost = (el) => {
    modelHandle = mountModel?.(el, { driver: untrack(() => driver), model: untrack(() => model) }, (next) => {
      driver = next.driver;
      model = next.model;
      if (next.model) level = '';
    });
    return () => modelHandle?.destroy?.();
  };

  async function save() {
    saving = true;
    try {
      await onsave?.({
        name: name.trim() || start.name,
        role,
        persona,
        driver,
        model: model.trim(),
        modelLevel: level,
        effort,
        approval,
        // Порог уходит, только если человек его двигал: иначе остаётся прежний режим.
        ...(touched ? { compactAtPercent: percent } : {}),
        monthlyTokenLimit: Math.max(0, Math.floor(Number(String(monthLimit).replace(/[\s_]/g, ''))) || 0) || null,
        skills: skills.filter((s) => chosen[s.id]).map((s) => s.id),
      });
    } finally {
      saving = false;
    }
  }
</script>

<div class="dp">
  <Section title={tr('Внешность')}>
    <div class="dp-look" {@attach lookHost}></div>
    <Switch
      label={tr('Питомец на рабочем столе')}
      bind:checked={pet}
      disabled={petBlocked && !pet}
      onchange={() => onpet?.(pet)}
    />
  </Section>

  <Section title={tr('Кто это')}>
    <Field label={tr('Имя')}>
      {#snippet children(f)}<TextInput id={f.id} bind:value={name} />{/snippet}
    </Field>
    <Field label={tr('Роль')}>
      {#snippet children(f)}<TextInput id={f.id} bind:value={role} />{/snippet}
    </Field>
    <Field label={tr('Характер и правила')}>
      {#snippet children(f)}<TextArea id={f.id} rows={10} autosize={false} bind:value={persona} />{/snippet}
    </Field>
  </Section>

  <Section title={tr('Модель')}>
    <div class="dp-model" {@attach modelHost}></div>
    <div class="dp-pair">
      <Field label={tr('Уровень модели')} hint={levelNote}>
        {#snippet children(f)}
          <Select id={f.id} describedby={f.describedby} options={levelOptions} value={level} onchange={(e) => pickLevel(e.currentTarget.value)} />
        {/snippet}
      </Field>
      <Field label={tr('Усилие')}>
        {#snippet children(f)}<Select id={f.id} options={effortOptions} bind:value={effort} />{/snippet}
      </Field>
    </div>
    <div class="dp-block dp-compact">
      <span class="dp-label">{tr('Память разговора')}</span>
      <Slider
        bind:value={percent}
        min={sliderMin}
        max={sliderMax}
        step={sliderStep}
        label={tr('Сжимать при {n}% лимита', { n: percent })}
        valueText={tr('{n}% лимита', { n: percent })}
        marks={COMPACT_MARKS}
        oninput={moved}
        aria-describedby="dp-compact-info"
      />
      <div id="dp-compact-info" class="dp-compact__info">
        <p class="dp-note dp-compact__tokens">{tokensLine}</p>
        {#if isDefault}
          <p class="dp-note">{tr('Пока действует прежний режим: ≈ {n} токенов. Подвинь ползунок, чтобы задать долю окна.', { n: fmt(info.currentTokens) })}</p>
        {/if}
        {#if capped}
          <p class="dp-note">{tr('Выше {n} токенов порог не поднимается: нужен запас под системный промпт и ответ.', { n: fmt(usable) })}</p>
        {/if}
        {#if costWarn}
          <p class="dp-note dp-compact__warn" role="note">{tr('Каждый ход будет отправлять до ≈ {n} токенов истории, это дорого. Для большинства задач хватает 30–50%.', { n: fmt(shownTokens) })}</p>
        {/if}
        <p class="dp-note">{tr('Чем выше порог, тем дороже каждый ход. Чем ниже, тем чаще бот забывает детали: старая часть переписки сворачивается в короткую сводку.')}</p>
      </div>
    </div>
    <Field label={tr('Лимит токенов в месяц')} hint={monthNote}>
      {#snippet children(f)}
        <TextInput id={f.id} describedby={f.describedby} type="text" inputmode="numeric" placeholder={tr('без лимита')} bind:value={monthLimit} />
      {/snippet}
    </Field>
  </Section>

  <Section title={tr('Разрешения')}>
    <Field label={tr('Спрашивать разрешение')}>
      {#snippet children(f)}<Select id={f.id} options={approvalOptions} bind:value={approval} />{/snippet}
    </Field>
    <div class="dp-block">
      <span class="dp-label">{tr('Сайты, где бот может действовать')}</span>
      {#if (bot.browserSites || []).length}
        <ul class="dp-sites">
          {#each bot.browserSites as site (site)}
            <li>
              <span>{site}</span>
              <button type="button" class="dp-x" title={tr('Забрать разрешение')} aria-label={tr('Забрать разрешение')} onclick={() => onrevokesite?.(site)}>
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="dp-note">{tr('Пока ни одного: бот спросит на первом действии.')}</p>
      {/if}
    </div>
    <div class="dp-google">
      <div>
        <span class="dp-label">{tr('Вход в Google в браузере бота')}</span>
        <span class="dp-note">{bot.googleSignIn === 'done' ? tr('выполнен') : tr('не выполнен')}</span>
      </div>
      <Button size="sm" onclick={() => ongoogle?.()}>{tr('Открыть браузер')}</Button>
    </div>
    <div class="dp-block">
      <span class="dp-label">{tr('Журнал разрешений')}</span>
      <ul class="activity-list dp-activity" id="pf-activity"></ul>
    </div>
  </Section>

  <Section title={tr('Навыки')}>
    {#if skills.length}
      <div class="dp-skills">
        {#each skills as skill (skill.id)}
          <span title={skill.description || ''}><Checkbox label={skill.name} bind:checked={chosen[skill.id]} /></span>
        {/each}
      </div>
    {:else}
      <p class="dp-note">{tr('Навыков пока нет. Их можно добавить в настройках или импортировать вместе с командой.')}</p>
    {/if}
  </Section>

  <div class="dp-save">
    <p class="dp-note">{tr('Папка бота')}: <code>{folder}</code></p>
    <Button variant="primary" loading={saving} onclick={save}>{tr('Сохранить')}</Button>
  </div>

  <Section title={tr('Удаление')} description={tr('Бот уйдёт вместе с чатом и памятью.')}>
    <div><Button variant="attention" onclick={() => ondelete?.()}>{tr('Удалить бота')}</Button></div>
  </Section>
</div>

<style>
  .dp { display: flex; flex-direction: column; gap: var(--k-space-5); }
  .dp :global(.k-section) { display: flex; flex-direction: column; gap: var(--k-space-3); }
  .dp-pair { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--k-space-3); }
  .dp-block { display: flex; flex-direction: column; gap: var(--k-space-2); }
  .dp-label { color: var(--k-text-2); font: 600 var(--k-text-xs) / 1.3 var(--k-font-body); }
  .dp-compact { gap: var(--k-space-2); margin-top: var(--k-space-2); }
  .dp-compact__info { display: flex; flex-direction: column; gap: var(--k-space-1); }
  .dp-compact__tokens { color: var(--k-text-2); }
  .dp-compact__warn { color: var(--k-attention-text); }
  .dp-note { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); }
  .dp-note code { font-family: var(--k-font-mono); font-size: 0.92em; overflow-wrap: anywhere; }
  .dp-sites {
    margin: 0;
    padding: 0 var(--k-space-3);
    list-style: none;
    border: 1px solid var(--k-line);
    border-radius: var(--k-radius-md);
  }
  .dp-sites li { display: flex; align-items: center; gap: var(--k-space-2); min-height: 40px; color: var(--k-text); font: 400 var(--k-text-sm) / 1.3 var(--k-font-body); }
  .dp-sites li + li { border-top: 1px solid var(--k-line); }
  .dp-sites span { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .dp-x {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--k-radius-pill);
    background: none;
    color: var(--k-text-3);
    cursor: pointer;
  }
  .dp-x:hover { background: var(--k-surface-2); color: var(--k-text); }
  .dp-x:focus-visible { outline: none; box-shadow: var(--k-focus); }
  .dp-google {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3);
    padding: var(--k-space-3) var(--k-space-4);
    border-radius: var(--k-radius-md);
    background: var(--k-surface-2);
  }
  .dp-google > div { display: flex; flex-direction: column; gap: 2px; }
  .dp-skills { display: flex; flex-direction: column; gap: var(--k-space-2); }
  .dp-skills :global(.k-check) { position: relative; }
  .dp-save {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3);
    padding: var(--k-space-3) 0;
    border-top: 1px solid var(--k-line);
    background: var(--k-surface);
  }
  .dp-save .dp-note { min-width: 0; }
  @media (max-width: 520px) {
    .dp-pair { grid-template-columns: 1fr; }
  }
</style>
