<script>
  // Вкладка «Профиль» в ящике бота: «Настройки бота» карточками кита (форма настроек, см. docs/design/kit.md).
  // Кнопки «Сохранить» нет: любая правка уходит сама (`onsave` → PATCH). Текст — через 600 мс после
  // последнего символа, выборы, переключатели и отпущенный ползунок — сразу; закрытие вкладки дописывает
  // отложенное. Конструктор внешности и выбор модели монтирует app.js в свои места (mountLook, mountModel):
  // у них свои движки аватаров и список провайдеров.
  import { untrack } from 'svelte';
  import SettingsCard from '../kit/SettingsCard.svelte';
  import FieldRow from '../kit/FieldRow.svelte';
  import InfoTip from '../kit/InfoTip.svelte';
  import TextInput from '../kit/TextInput.svelte';
  import TextArea from '../kit/TextArea.svelte';
  import Select from '../kit/Select.svelte';
  import Slider from '../kit/Slider.svelte';
  import Switch from '../kit/Switch.svelte';
  import Checkbox from '../kit/Checkbox.svelte';
  import RadioGroup from '../kit/RadioGroup.svelte';
  import Button from '../kit/Button.svelte';
  import IconButton from '../kit/IconButton.svelte';
  import Modal from '../kit/Modal.svelte';
  import Icon from '../kit/Icon.svelte';
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

  // Как в src/catalog.rs (PERSONA_MAX): длиннее инструкции бот не получит.
  const PERSONA_MAX = 12000;
  const TEXT_DELAY = 600;

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
  // Счётчик выбора внешности: сам выбор лежит у app.js, здесь нужно только знать, что он изменился.
  let lookTick = $state(0);
  let status = $state('idle');
  let confirmOpen = $state(false);
  let openSites = $state(false);
  let openActivity = $state(false);

  // app.js сообщает, что сохранилось на самом деле (стол мог быть полон или запрос упал).
  $effect(() => {
    if (petOn !== undefined) pet = petOn;
  });

  const levelOptions = $derived([
    { value: '', label: tr('Авто') },
    { value: 'strong', label: tr('Сильная') },
    { value: 'standard', label: tr('Обычная') },
    { value: 'fast', label: tr('Быстрая') },
  ]);
  const effortOptions = $derived([
    { value: 'medium', label: tr('Обычное') },
    { value: 'high', label: tr('Высокое') },
    { value: 'low', label: tr('Низкое') },
  ]);
  // Что делает каждый режим, см. policy_needs_approval и needs_approval_for в src/tools.rs.
  const approvalOptions = $derived([
    { value: 'strict', label: tr('Перед каждым действием'), hint: tr('Читать может сам. Всё, что что-то меняет, делает только с твоего «да».') },
    { value: 'ask', label: tr('Только за пределами своей папки'), hint: tr('В своей папке работает сам. Чужие файлы, опасные команды, новые сайты и платные картинки — спросит.') },
    { value: 'auto', label: tr('Никогда'), hint: tr('Делает всё сам, даже вне своей папки. Ключи, выпуск наружу (push) и рабочие процессы всё равно спросит.') },
  ]);
  // Если порог не задан, ползунок стоит там, где его держит старый режим (по ответу сервера).
  $effect(() => {
    if (!touched && info && !info.stored && Number.isFinite(Number(info.percent))) percent = Number(info.percent);
  });
  const sliderMin = $derived(info?.min ?? 20);
  const sliderMax = $derived(info?.max ?? 95);
  const sliderStep = $derived(info?.step ?? 5);
  const sliderMarks = $derived([sliderMin, ...COMPACT_MARKS, sliderMax]);
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
  // Все пояснения к памяти разговора лежат в подсказке «i»; на виду остаётся только предупреждение о цене.
  const memoryTip = $derived(
    [
      tokensLine,
      isDefault ? tr('Пока действует прежний режим: ≈ {n} токенов. Подвинь ползунок, чтобы задать долю окна.', { n: fmt(info.currentTokens) }) : '',
      capped ? tr('Выше {n} токенов порог не поднимается: нужен запас под системный промпт и ответ.', { n: fmt(usable) }) : '',
      tr('Чем выше порог, тем дороже каждый ход. Чем ниже, тем чаще бот забывает детали: старая часть переписки сворачивается в короткую сводку.'),
    ]
      .filter(Boolean)
      .join('\n'),
  );
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

  const sites = $derived(bot.browserSites || []);
  const sitesLine = $derived(sites.length ? tr('Разрешено сайтов: {n}', { n: sites.length }) : tr('Пока ни одного. Бот спросит, когда понадобится.'));
  const googleDone = $derived(bot.googleSignIn === 'done');
  const personaOver = $derived(persona.length > PERSONA_MAX * 0.9);

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

  const lookHost = (el) => mountLook?.(el, () => lookTick++);
  const modelHost = (el) => {
    modelHandle = mountModel?.(el, { driver: untrack(() => driver), model: untrack(() => model) }, (next) => {
      driver = next.driver;
      model = next.model;
      if (next.model) level = '';
    });
    return () => modelHandle?.destroy?.();
  };

  // ---- автосохранение ------------------------------------------------------------

  function payload() {
    return {
      // Пустое имя не сохраняем: бот остаётся с прежним.
      ...(name.trim() ? { name: name.trim() } : {}),
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
    };
  }
  const snapshot = $derived(JSON.stringify([payload(), lookTick]));
  const textKey = $derived(JSON.stringify([name, role, persona, monthLimit]));

  let lastSaved = untrack(() => snapshot);
  let lastText = untrack(() => textKey);
  let timer = null;
  let pending = false;
  let inflight = Promise.resolve();

  function schedule(delay) {
    pending = true;
    status = 'saving';
    clearTimeout(timer);
    timer = setTimeout(run, delay);
  }

  function run() {
    clearTimeout(timer);
    timer = null;
    pending = false;
    const snap = snapshot;
    if (snap === lastSaved) {
      status = 'saved';
      return inflight;
    }
    const body = payload();
    inflight = inflight.then(async () => {
      let ok = true;
      try {
        ok = (await onsave?.(body)) !== false;
      } catch {
        ok = false;
      }
      if (ok) lastSaved = snap;
      if (!pending) status = ok ? 'saved' : 'error';
    });
    return inflight;
  }

  // Правка изменила данные: текст ждёт паузы в наборе, всё остальное уходит сразу.
  $effect(() => {
    const now = snapshot;
    const text = textKey;
    if (now === lastSaved) return;
    const typing = text !== lastText;
    lastText = text;
    schedule(typing ? TEXT_DELAY : 0);
  });

  // Ползунок отпустили, вкладку закрыли или переключили: дописываем то, что ещё ждёт.
  function flushNow() {
    if (timer) run();
  }
  $effect(() => () => flushNow());

  const statusText = $derived(
    status === 'saving' ? tr('Сохраняю…') : status === 'saved' ? tr('Сохранено') : status === 'error' ? tr('Не удалось сохранить') : tr('Изменения сохраняются сами'),
  );

  async function confirmDelete() {
    confirmOpen = false;
    // Бота не станет: отложенная правка ему уже не нужна.
    clearTimeout(timer);
    timer = null;
    pending = false;
    await ondelete?.();
  }
</script>

<div class="k-sform dp">
  <header class="dp-head">
    <div class="dp-head__text">
      <h2 class="dp-head__title">{tr('Настройки бота')}</h2>
      <span class="dp-status" class:is-fresh={status === 'saving' || status === 'saved'} class:is-error={status === 'error'} aria-live="polite">
        <Icon name={status === 'error' ? 'alert-circle' : 'check'} size={14} />
        <span>{statusText}</span>
      </span>
    </div>
    <IconButton label={tr('Удалить бота')} class="dp-trash" onclick={() => (confirmOpen = true)}>
      {#snippet icon()}<Icon name="trash" size={18} />{/snippet}
    </IconButton>
  </header>
  <p class="dp-folder"><Icon name="folder" size={15} /><code title={folder}>{folder}</code></p>

  <SettingsCard title={tr('Внешность')} icon="eye">
    <div class="dp-look" {@attach lookHost}></div>
    <Switch
      label={tr('Питомец на рабочем столе')}
      bind:checked={pet}
      disabled={petBlocked && !pet}
      onchange={() => onpet?.(pet)}
    />
  </SettingsCard>

  <SettingsCard title={tr('Кто это')} description={tr('Роль и стиль работы.')} icon="bot">
    <FieldRow label={tr('Имя')} icon="user-round">
      {#snippet children(f)}<TextInput id={f.id} bind:value={name} />{/snippet}
    </FieldRow>
    <FieldRow label={tr('Чем занимается')} icon="target">
      {#snippet children(f)}<TextInput id={f.id} bind:value={role} />{/snippet}
    </FieldRow>
    <FieldRow
      block
      label={tr('Инструкции')}
      icon="layers"
      tip={tr('Бот читает это перед каждым ответом. Пиши, как он должен работать: стиль, чего избегать, куда сохранять результат.')}
      tipLabel={tr('Что писать в инструкциях')}
    >
      {#snippet children(f)}
        <div class="dp-persona-box">
          <TextArea
            id={f.id}
            class="dp-persona"
            bind:value={persona}
            maxlength={PERSONA_MAX}
            placeholder={tr('Пиши коротко и по делу. Избегай общих фраз и клише. Сохраняй черновики в файлы рабочей папки.')}
          />
          <div class="dp-under">
            <span>{tr('Опишите стиль, ограничения и действия бота.')}</span>
            <span class="dp-count" class:is-over={personaOver}>{fmt(persona.length)} / {fmt(PERSONA_MAX)}</span>
          </div>
        </div>
      {/snippet}
    </FieldRow>
  </SettingsCard>

  <SettingsCard title={tr('Модель')} icon="cpu">
    <div class="dp-model" {@attach modelHost}></div>
    <div class="k-fpair">
      <FieldRow stack label={tr('Уровень модели')} icon="sliders" tip={levelNote} tipLabel={tr('Какая модель сейчас')}>
        {#snippet children(f)}
          <Select id={f.id} options={levelOptions} value={level} onchange={(e) => pickLevel(e.currentTarget.value)} />
        {/snippet}
      </FieldRow>
      <FieldRow stack label={tr('Усилие')} icon="zap">
        {#snippet children(f)}<Select id={f.id} options={effortOptions} bind:value={effort} />{/snippet}
      </FieldRow>
    </div>
    <div class="k-inset dp-compact">
      <div class="k-frow__label">
        <span class="k-tile k-tile--sm" aria-hidden="true"><Icon name="chat-bubble" size={null} /></span>
        <span>{tr('Память разговора')}</span>
        <InfoTip text={memoryTip} label={tr('Сколько это токенов')} />
      </div>
      <Slider
        bind:value={percent}
        min={sliderMin}
        max={sliderMax}
        step={sliderStep}
        label={tr('Сжимать при {n}% лимита', { n: percent })}
        valueText={tr('{n}% лимита', { n: percent })}
        marks={sliderMarks}
        oninput={moved}
        onchange={flushNow}
      />
      {#if costWarn}
        <p class="dp-note dp-compact__warn" role="note">{tr('Каждый ход будет отправлять до ≈ {n} токенов истории, это дорого. Для большинства задач хватает 30–50%.', { n: fmt(shownTokens) })}</p>
      {/if}
    </div>
    <FieldRow label={tr('Лимит токенов в месяц')} icon="database" tip={monthNote} tipLabel={tr('Сколько потрачено в этом месяце')}>
      {#snippet children(f)}
        <TextInput id={f.id} type="text" inputmode="numeric" placeholder={tr('без лимита')} bind:value={monthLimit} />
      {/snippet}
    </FieldRow>
  </SettingsCard>

  <SettingsCard title={tr('Разрешения')} icon="shield-check">
    <RadioGroup variant="cards" name="dp-approval" legend={tr('Когда бот спрашивает тебя')} options={approvalOptions} bind:value={approval} />

    <ul class="dp-links">
      <li>
        {#if sites.length}
          <button type="button" class="dp-link" aria-expanded={openSites} aria-controls="dp-sites" onclick={() => (openSites = !openSites)}>
            <span class="k-tile k-tile--sm" aria-hidden="true"><Icon name="link" size={null} /></span>
            <span class="dp-link__text"><b>{tr('Сайты')}</b><small>{sitesLine}</small></span>
            <Icon name="chevron-right" size={16} class={['dp-chev', openSites && 'is-open']} />
          </button>
          <div class="dp-more" id="dp-sites" hidden={!openSites}>
            <ul class="dp-sites">
              {#each sites as site (site)}
                <li>
                  <span>{site}</span>
                  <button type="button" class="dp-x" title={tr('Забрать разрешение')} aria-label={tr('Забрать разрешение')} onclick={() => onrevokesite?.(site)}>
                    <Icon name="close" size={14} stroke={2.2} />
                  </button>
                </li>
              {/each}
            </ul>
          </div>
        {:else}
          <div class="dp-link is-static">
            <span class="k-tile k-tile--sm" aria-hidden="true"><Icon name="link" size={null} /></span>
            <span class="dp-link__text"><b>{tr('Сайты')}</b><small>{sitesLine}</small></span>
          </div>
        {/if}
      </li>
      <li>
        <button type="button" class="dp-link" onclick={() => ongoogle?.()}>
          <span class="k-tile k-tile--sm" aria-hidden="true"><Icon name="user-round" size={null} /></span>
          <span class="dp-link__text">
            <b>{tr('Аккаунт Google')}</b>
            <small>{googleDone ? tr('Бот вошёл в свой браузер.') : tr('Бот не вошёл. Нужно, чтобы он работал в Почте и Диске.')}</small>
          </span>
          <span class="dp-link__act">{googleDone ? tr('Открыть') : tr('Войти')}</span>
        </button>
      </li>
      <li>
        <button type="button" class="dp-link" aria-expanded={openActivity} aria-controls="dp-activity" onclick={() => (openActivity = !openActivity)}>
          <span class="k-tile k-tile--sm" aria-hidden="true"><Icon name="file-lines" size={null} /></span>
          <span class="dp-link__text"><b>{tr('Что бот делал')}</b><small>{tr('По правилам и по твоим ответам на карточки.')}</small></span>
          <Icon name="chevron-right" size={16} class={['dp-chev', openActivity && 'is-open']} />
        </button>
        <div class="dp-more" id="dp-activity" hidden={!openActivity}>
          <ul class="activity-list dp-activity" id="pf-activity"></ul>
        </div>
      </li>
    </ul>
  </SettingsCard>

  <SettingsCard title={tr('Навыки')} icon="puzzle">
    {#if skills.length}
      <div class="dp-skills">
        {#each skills as skill (skill.id)}
          <span title={skill.description || ''}><Checkbox label={skill.name} bind:checked={chosen[skill.id]} /></span>
        {/each}
      </div>
    {:else}
      <p class="dp-empty"><Icon name="puzzle" size={16} />{tr('Навыков пока нет. Их можно добавить в настройках или импортировать вместе с командой.')}</p>
    {/if}
  </SettingsCard>
</div>

<Modal bind:open={confirmOpen} size="sm" title={tr('Удалить бота «{name}»?', { name: name.trim() || start.name })} description={tr('Вы уверены, что хотите удалить бота и всю его историю? Чат, память, рутины и входы в сайты удалятся, вернуть их не получится. Папка с файлами бота останется на диске.')}>
  {#snippet footer()}
    <Button data-autofocus onclick={() => (confirmOpen = false)}>{tr('Отмена')}</Button>
    <Button variant="attention" onclick={confirmDelete}>{tr('Удалить')}</Button>
  {/snippet}
</Modal>

<style>
  .dp-head { display: flex; align-items: center; gap: var(--k-space-1); padding: var(--k-space-1) var(--k-space-1) 0 var(--k-space-2); }
  .dp-head__text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .dp-head__title { margin: 0; font: 600 var(--k-text-xl) / 1.2 var(--k-font-display); color: var(--k-text); }
  .dp-status { display: inline-flex; align-items: center; gap: 6px; color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.3 var(--k-font-body); transition: color var(--k-dur-fast) var(--k-ease-out); }
  .dp-status.is-fresh { color: var(--k-text-2); }
  .dp-status.is-error { color: var(--k-attention-text); }
  .dp :global(.dp-trash) { color: var(--k-text-2); }
  .dp :global(.dp-trash:hover) { background: var(--k-attention-tint); color: var(--k-attention-text); }
  .dp-folder { margin: 0; display: flex; align-items: center; gap: var(--k-space-2); min-width: 0; padding: 0 var(--k-space-2); color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.3 var(--k-font-body); }
  .dp-folder :global(svg) { flex: none; }
  .dp-folder code { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 4px 8px; border-radius: var(--k-radius-xs); background: var(--k-surface); color: var(--k-text-2); font: 400 var(--k-text-xs) / 1.3 var(--k-font-mono); }

  .dp-persona-box { display: flex; flex-direction: column; gap: var(--k-space-2); }
  .dp :global(.k-textarea.dp-persona) {
    width: 100%;
    min-height: 220px;
    max-height: 480px;
    padding: var(--k-space-4);
    font-size: 15px;
    line-height: 1.55;
    overflow-y: auto;
    overflow-wrap: anywhere;
    resize: none;
  }
  .dp-under { display: flex; justify-content: space-between; gap: var(--k-space-3); color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); }
  .dp-count { white-space: nowrap; font-variant-numeric: tabular-nums; }
  .dp-count.is-over { color: var(--k-attention-text); }

  .dp-compact__warn { color: var(--k-attention-text); }
  .dp-note { margin: 0; color: var(--k-text-3); font: 400 var(--k-text-sm) / var(--k-leading-body) var(--k-font-body); }

  .dp-links { margin: 0; padding: 0; list-style: none; border: 1px solid var(--k-line); border-radius: var(--k-radius-md); overflow: hidden; }
  .dp-links > li + li { border-top: 1px solid var(--k-line); }
  .dp-link {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    padding: var(--k-space-3);
    border: 0;
    background: none;
    color: var(--k-text);
    text-align: left;
    font: inherit;
    cursor: pointer;
    transition: background var(--k-dur-fast) var(--k-ease-out);
  }
  .dp-link:hover { background: var(--k-surface-2); }
  .dp-link:focus-visible { outline: none; box-shadow: inset var(--k-focus); }
  .dp-link.is-static { cursor: default; }
  .dp-link.is-static:hover { background: none; }
  .dp-link__text { flex: 1; min-width: 0; }
  .dp-link__text b { display: block; font: 600 var(--k-text-sm) / 1.3 var(--k-font-body); }
  .dp-link__text small { display: block; color: var(--k-text-3); font: 400 var(--k-text-xs) / 1.4 var(--k-font-body); }
  .dp-link__act { flex: none; color: var(--k-brand-text); font: 600 var(--k-text-sm) / 1 var(--k-font-body); }
  .dp-link :global(.dp-chev) { flex: none; color: var(--k-text-3); transition: transform var(--k-dur-fast) var(--k-ease-out); }
  .dp-link :global(.dp-chev.is-open) { transform: rotate(90deg); }
  .dp-more { padding: 0 var(--k-space-3) var(--k-space-3); }
  .dp-more[hidden] { display: none; }
  .dp-activity { margin: 0; padding: 0; list-style: none; max-height: 240px; overflow-y: auto; color: var(--k-text-2); font: 400 var(--k-text-xs) / 1.5 var(--k-font-body); }
  .dp-sites { margin: 0; padding: 0; list-style: none; }
  .dp-sites li { display: flex; align-items: center; gap: var(--k-space-2); min-height: 36px; color: var(--k-text); font: 400 var(--k-text-sm) / 1.3 var(--k-font-body); }
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

  .dp-skills { display: flex; flex-direction: column; gap: var(--k-space-2); }
  .dp-skills :global(.k-check) { position: relative; }
  .dp-empty { margin: 0; display: flex; align-items: center; gap: var(--k-space-3); padding: var(--k-space-3) var(--k-space-4); border-radius: var(--k-radius-md); background: var(--k-surface-2); color: var(--k-text-2); font: 400 var(--k-text-sm) / 1.4 var(--k-font-body); }
  .dp-empty :global(svg) { flex: none; }
</style>
