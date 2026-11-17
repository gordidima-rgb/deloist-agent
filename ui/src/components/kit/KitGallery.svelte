<script>
  // Витрина базовых компонентов кита для /ui-demo.html (только debug-сборка).
  // Один и тот же набор показан в двух колонках: data-theme="light" и data-theme="dark".
  import { tr } from '../../lib/i18n.js';
  import { toast } from '../../lib/toast.js';
  import Button from './buttons/Button.svelte';
  import IconButton from './buttons/IconButton.svelte';
  import Field from './inputs/Field.svelte';
  import TextInput from './inputs/TextInput.svelte';
  import TextArea from './inputs/TextArea.svelte';
  import Select from './inputs/Select.svelte';
  import Switch from './choice/Switch.svelte';
  import Checkbox from './choice/Checkbox.svelte';
  import RadioGroup from './choice/RadioGroup.svelte';
  import Slider from './inputs/Slider.svelte';
  import Modal from './overlay/Modal.svelte';
  import Section from './layout/Section.svelte';
  import Row from './layout/Row.svelte';
  import ListItem from './layout/ListItem.svelte';
  import ListRow from './layout/ListRow.svelte';
  import Menu from './overlay/Menu.svelte';
  import Avatar from './media/Avatar.svelte';
  import Tabs from './choice/Tabs.svelte';
  import Chip from './status/Chip.svelte';
  import Badge from './status/Badge.svelte';
  import EmptyState from './status/EmptyState.svelte';
  import Spinner from './status/Spinner.svelte';
  import Skeleton from './status/Skeleton.svelte';
  import Tooltip from './overlay/Tooltip.svelte';
  import Toast from './overlay/Toast.svelte';
  import SplitView from './layout/SplitView.svelte';
  import Toolbar from './layout/Toolbar.svelte';

  import Icon from './media/Icon.svelte';
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
  <Icon name="gear-simple" size={null} stroke={1.8} />
{/snippet}

{#snippet plusIcon()}
  <Icon name="plus" size={null} />
{/snippet}

{#snippet dotsIcon()}
  <Icon name="more" size={null} stroke={2.5} />
{/snippet}

{#snippet emptyIcon()}
  <Icon name="box" size={null} stroke={1.7} />
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
    <Button variant="danger">{tr('Удалить')}</Button>
    <Button variant="danger-solid">{tr('Да, удалить')}</Button>
    <Button disabled>{tr('Недоступна')}</Button>
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
    <Chip selected={chip === 'all'} onclick={() => (chip = 'all')} color="var(--k-bot-sky)">{tr('Все')}</Chip>
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
  <div class="gallery__row">
    <Badge tone="attention" text={tr('Ждёт тебя')} />
    <Badge tone="ai" text={tr('Работает')} />
    <Badge tone="done" text={tr('Готово')} />
    <Badge tone="sleep" text={tr('Спит')} />
    <Badge tone="error" text={tr('Ошибка')} />
    <Badge tone="outline" text={tr('Бета')} />
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
  <Toast icon="check" text={tr('Профиль сохранён')} action={{ label: tr('Отменить'), onclick: () => {} }} />
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

{#snippet menuStage()}
  <div class="gallery__menu">
    <Menu
      label={tr('Действия')}
      items={[
        { id: 'copy', label: tr('Копировать'), icon: 'copy', shortcut: 'Ctrl+C' },
        { id: 'pin', label: tr('Закрепить'), icon: 'pin', kind: 'check', checked: true },
        { id: 's1', kind: 'sep' },
        { id: 'del', label: tr('Удалить'), icon: 'trash', danger: true },
      ]}
    />
  </div>
  <ul class="k-lrows">
    <ListRow icon="link" title={tr('Строка со стрелкой')} sub={tr('Раскрывается по клику')} variant="chevron">
      {#snippet children()}<p>{tr('Тело строки растёт по высоте.')}</p>{/snippet}
    </ListRow>
    <ListRow icon="user" title={tr('Строка с действием')} sub={tr('Кнопка справа, строка не нажимается')} variant="action" actionLabel={tr('Войти')} />
    <ListRow icon="gear" title={tr('Строка с переключателем')} variant="switch" checked />
  </ul>
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
    <h2>{tr('Меню и строки')}</h2>
    <div class="gallery__pair">
      {#each themes as theme (theme)}
        <div class="gallery__col" data-theme={theme}>
          <div class="gallery__tag">{themeName(theme)}</div>
          <div class="gallery__stage">{@render menuStage()}</div>
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
