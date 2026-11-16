<script>
  // Витрина базовых компонентов кита для /ui-demo.html (только debug-сборка).
  // Один и тот же набор показан в двух колонках: data-theme="light" и data-theme="dark".
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import Button from './Button.svelte';
  import IconButton from './IconButton.svelte';
  import Field from './Field.svelte';
  import TextInput from './TextInput.svelte';
  import TextArea from './TextArea.svelte';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import Checkbox from './Checkbox.svelte';
  import RadioGroup from './RadioGroup.svelte';
  import Slider from './Slider.svelte';
  import Modal from './Modal.svelte';
  import Section from './Section.svelte';
  import Row from './Row.svelte';
  import ListItem from './ListItem.svelte';
  import Avatar from './Avatar.svelte';
  import Tabs from './Tabs.svelte';
  import Chip from './Chip.svelte';
  import Badge from './Badge.svelte';
  import EmptyState from './EmptyState.svelte';
  import Spinner from './Spinner.svelte';
  import Skeleton from './Skeleton.svelte';
  import Tooltip from './Tooltip.svelte';
  import Toast from './Toast.svelte';
  import SplitView from './SplitView.svelte';
  import Toolbar from './Toolbar.svelte';

  const themes = ['light', 'dark'];
  const photo = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 40 40'%3E%3Crect width='40' height='40' fill='%234fb4ff'/%3E%3Ccircle cx='20' cy='16' r='7' fill='%23ffffff'/%3E%3C/svg%3E";

  let name = $state(tr('Скаут'));
  let role = $state('');
  let model = $state('default');
  let readAloud = $state(true);
  let showInDock = $state(true);
  let autoUpdate = $state(true);
  let answerer = $state('lead');
  let brightness = $state(60);
  let activeBot = $state('mira');
  let tab = $state('chat');
  let chip = $state('all');
  let modalOpen = $state({ light: false, dark: false });
  let sheetOpen = $state({ light: false, dark: false });

  function announce() {
    toast(tr('Профиль сохранён'), { action: { label: tr('Отменить'), onclick: () => {} } });
  }

  const themeName = (theme) => (theme === 'light' ? tr('Светлая тема') : tr('Тёмная тема'));
</script>

{#snippet gearIcon()}
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="3" /><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3 1a7 7 0 0 0-2-1.2L14.2 3h-4.4l-.4 2.7a7 7 0 0 0-2 1.2l-2.3-1-2 3.4 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.3-1a7 7 0 0 0 2 1.2l.4 2.7h4.4l.4-2.7a7 7 0 0 0 2-1.2l2.3 1 2-3.4-2-1.5c.1-.4.1-.8.1-1.2Z" />
  </svg>
{/snippet}

{#snippet plusIcon()}
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
{/snippet}

{#snippet dotsIcon()}
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="12" r="1" /></svg>
{/snippet}

{#snippet emptyIcon()}
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M4 7.5 12 3l8 4.5v9L12 21l-8-4.5z" /><path d="M4 7.5 12 12l8-4.5M12 12v9" />
  </svg>
{/snippet}

{#snippet emptyAction()}
  <Button size="sm" variant="primary" onclick={announce}>{tr('Запустить сейчас')}</Button>
{/snippet}

{#snippet settingsActions()}
  <Button size="sm" variant="ghost">{tr('Проверить')}</Button>
{/snippet}

{#snippet layoutActions()}
  <Button size="sm" variant="primary">{tr('Добавить')}</Button>
{/snippet}

{#snippet layoutSidebar()}
  <ul class="k-list">
    <ListItem title={tr('Мира')} subtitle={tr('Ищет источники')} active onclick={() => (activeBot = 'mira')} />
    <ListItem title={tr('Скаут')} subtitle={tr('Проверяет факты')} onclick={() => (activeBot = 'scout')} />
    <ListItem title={tr('Аня')} subtitle={tr('Готовит отчёт')} onclick={() => (activeBot = 'anya')} />
  </ul>
{/snippet}

{#snippet buttonsStage()}
  <div class="gallery__row">
    <Button variant="primary">{tr('Основная')}</Button>
    <Button>{tr('Обычная')}</Button>
    <Button variant="ghost">{tr('Тихая')}</Button>
    <Button variant="ink">{tr('Тушь')}</Button>
    <Button variant="attention">{tr('Внимание')}</Button>
  </div>
  <div class="gallery__row">
    <Button size="sm">{tr('Малая')}</Button>
    <Button loading>{tr('Загружаю…')}</Button>
    <Button disabled>{tr('Недоступна')}</Button>
    <IconButton icon={gearIcon} label={tr('Настройки')} />
    <IconButton icon={plusIcon} label={tr('Добавить')} variant="ink" />
  </div>
{/snippet}

{#snippet formsStage()}
  <Field label={tr('Название бота')} hint={tr('Так его увидят в доке')}>
    {#snippet children({ id, describedby, invalid })}
      <TextInput {id} {describedby} {invalid} bind:value={name} placeholder={tr('Например, Скаут')} />
    {/snippet}
  </Field>
  <Field label={tr('Название бота')} error={tr('Имя уже занято: выберите другое.')}>
    {#snippet children({ id, describedby, invalid })}
      <TextInput {id} {describedby} {invalid} value={name} />
    {/snippet}
  </Field>
  <Field label={tr('С чем помогает')}>
    {#snippet children({ id })}
      <TextArea {id} bind:value={role} placeholder={tr('Коротко о роли')} />
    {/snippet}
  </Field>
  <Field label={tr('Модель')}>
    {#snippet children({ id })}
      <Select
        {id}
        bind:value={model}
        options={[
          { value: 'default', label: tr('как в настройках') },
          { value: 'flash', label: 'DeepSeek Flash' },
          { value: 'terra', label: 'GPT-6.1 Terra' },
        ]}
      />
    {/snippet}
  </Field>
  <Switch bind:checked={readAloud} label={tr('Озвучивать ответы')} />
  <Checkbox bind:checked={showInDock} label={tr('Показывать в доке')} />
  <RadioGroup
    bind:value={answerer}
    legend={tr('Кто отвечает')}
    options={[
      { value: 'lead', label: tr('Ведущий') },
      { value: 'auto', label: tr('Модель сама') },
    ]}
  />
  <Slider bind:value={brightness} label={tr('Яркость')} showValue />
{/snippet}

{#snippet settingsStage()}
  <Section title={tr('Обновления')} description={tr('Бот сам проверит и скачает новую версию.')} actions={settingsActions}>
    <Row label={tr('Проверять автоматически')} sublabel={tr('Раз в сутки, когда компьютер свободен')}>
      <Switch bind:checked={autoUpdate} />
    </Row>
    <Row label={tr('Установить ночью')} sublabel={tr('Раз в сутки, когда компьютер свободен')}>
      <Select
        value="yes"
        options={[
          { value: 'yes', label: tr('как в настройках') },
          { value: 'no', label: tr('Отмена') },
        ]}
      />
    </Row>
  </Section>
{/snippet}

{#snippet listStage()}
  <ul class="k-list">
    <ListItem
      title={tr('Мира')}
      subtitle={tr('Ищет источники')}
      meta="09:41"
      badge={3}
      pinned
      active={activeBot === 'mira'}
      onclick={() => (activeBot = 'mira')}
    >
      {#snippet avatar()}<Avatar name={tr('Мира')} size={36} ring="#ff8a3d" />{/snippet}
      {#snippet context()}<IconButton icon={dotsIcon} label={tr('Настройки')} size="sm" />{/snippet}
    </ListItem>
    <ListItem
      title={tr('Скаут')}
      subtitle={tr('Проверяет факты')}
      meta="08:10"
      unread
      active={activeBot === 'scout'}
      onclick={() => (activeBot = 'scout')}
    >
      {#snippet avatar()}<Avatar name={tr('Скаут')} emoji="🛰️" size={36} ring="#4fb4ff" />{/snippet}
      {#snippet context()}<IconButton icon={dotsIcon} label={tr('Настройки')} size="sm" />{/snippet}
    </ListItem>
    <ListItem
      title={tr('Аня')}
      subtitle={tr('Готовит отчёт')}
      meta={tr('Вчера')}
      badge={12}
      tone="attention"
      active={activeBot === 'anya'}
      onclick={() => (activeBot = 'anya')}
    >
      {#snippet avatar()}<Avatar name={tr('Аня')} emoji="🪄" size={36} ring="#a98bff" />{/snippet}
    </ListItem>
  </ul>
{/snippet}

{#snippet avatarStage()}
  <div class="gallery__row">
    <Avatar name={tr('Мира')} size={56} ring="#ff8a3d" />
    <Avatar name={tr('Скаут')} size={40} ring />
    <Avatar name={tr('Аня')} size={32} />
    <Avatar emoji="🦊" size={40} />
    <Avatar initials="AB" size={40} />
    <Avatar src={photo} size={40} label={tr('Скаут')} />
  </div>
{/snippet}

{#snippet tabsStage()}
  <Tabs bind:value={tab} label={tr('Вкладки, фишки, значки')} items={[{ id: 'chat', label: tr('Чат') }, { id: 'files', label: tr('Файлы') }, { id: 'reports', label: tr('Отчёты') }]} />
  <div class="gallery__row">
    <Chip selected={chip === 'all'} onclick={() => (chip = 'all')} color="#4fb4ff">{tr('Все')}</Chip>
    <Chip selected={chip === 'bots'} onclick={() => (chip = 'bots')}>{tr('Боты')}</Chip>
    <Chip selected={chip === 'groups'} onclick={() => (chip = 'groups')}>{tr('Команды')}</Chip>
    <Chip selected>{tr('Выбранная')}</Chip>
  </div>
  <div class="gallery__row">
    <Badge count={12} tone="brand" />
    <Badge count={150} max={99} tone="neutral" />
    <Badge text="AI" tone="ai" />
    <Badge dot tone="attention" label={tr('Внимание')} />
    <Badge text="!" tone="attention" />
  </div>
{/snippet}

{#snippet loadingStage()}
  <EmptyState icon={emptyIcon} title={tr('пока пусто')} text={tr('Здесь появятся отчёты, когда бот закончит работу.')} action={emptyAction} />
  <div class="gallery__row">
    <Spinner />
    <Spinner size="sm" />
    <Skeleton width={180} height={12} />
    <Skeleton circle width={40} height={40} />
    <Tooltip text={tr('Откроется в новом окне')}>
      <Button size="sm">{tr('Открыть окно')}</Button>
    </Tooltip>
  </div>
{/snippet}

{#snippet toastStage()}
  <div class="gallery__row">
    <Button onclick={announce}>{tr('Показать уведомление')}</Button>
  </div>
  <Toast text={tr('Профиль сохранён')} action={{ label: tr('Отменить'), onclick: () => {} }} />
{/snippet}

{#snippet dialogsStage(theme)}
  <div class="gallery__row">
    <Button onclick={() => (modalOpen[theme] = true)}>{tr('Открыть окно')}</Button>
    <Button variant="ghost" onclick={() => (sheetOpen[theme] = true)}>{tr('Открыть шторку')}</Button>
  </div>
  <Modal
    open={modalOpen[theme]}
    onclose={() => (modalOpen[theme] = false)}
    title={tr('Новый бот')}
    description={tr('Сохраню изменения в профиле бота.')}
  >
    <p>{tr('Сохраню изменения в профиле бота.')}</p>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => (modalOpen[theme] = false)}>{tr('Отмена')}</Button>
      <Button variant="primary" onclick={() => (modalOpen[theme] = false)}>{tr('Создать')}</Button>
    {/snippet}
  </Modal>
  <Modal variant="sheet" open={sheetOpen[theme]} onclose={() => (sheetOpen[theme] = false)} title={tr('Шторка снизу')}>
    <p>{tr('Сохраню изменения в профиле бота.')}</p>
  </Modal>
{/snippet}

{#snippet layoutStage()}
  <SplitView width="200px" sidebar={layoutSidebar}>
    <Toolbar title={tr('Боты')} subtitle={tr('Команда из трёх ботов')} actions={layoutActions} />
    <p class="gallery__note">{tr('Здесь будет чат выбранного бота.')}</p>
  </SplitView>
{/snippet}

<div class="gallery">
  <header class="gallery__head">
    <h1>{tr('Витрина кита: базовые компоненты')}</h1>
    <p>{tr('Все компоненты кита в обеих темах. Наведи, нажми, пройди Tab и стрелками: фокус виден.')}</p>
  </header>

  <section class="gallery__section">
    <h2>{tr('Кнопки')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render buttonsStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Поля и переключатели')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render formsStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Окна')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render dialogsStage(theme)}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Раздел и строка')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render settingsStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Список')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render listStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Аватары')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render avatarStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Вкладки, фишки, значки')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render tabsStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Пустота и загрузка')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render loadingStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Уведомления')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render toastStage()}</div>
        </div>
      {/each}
    </div>
  </section>

  <section class="gallery__section">
    <h2>{tr('Раскладка')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render layoutStage()}</div>
        </div>
      {/each}
    </div>
  </section>
</div>
