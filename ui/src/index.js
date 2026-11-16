// Точка входа: window.DeloistUI с помощниками mount*() для старых страниц.
// Новый компонент: файл в src/components, функция mountX здесь. См. docs/ui-svelte.md.
import { mountComponent } from './lib/mount.svelte.js';
import LimitRing from './components/LimitRing.svelte';
import ModelPicker from './components/ModelPicker.svelte';
import NewBotModel from './components/NewBotModel.svelte';
import ChatModelPopover from './components/ChatModelPopover.svelte';
import Catalog from './components/catalog/Catalog.svelte';
import Dock from './components/Dock.svelte';
import StepPill from './components/StepPill.svelte';
import MicButton from './components/MicButton.svelte';
import Menu from './components/Menu.svelte';
import MentionPopover from './components/MentionPopover.svelte';
import ChatDates from './components/chat/ChatDates.svelte';
import KnowMe from './components/KnowMe.svelte';
import VoiceChat from './components/VoiceChat.svelte';
import ApprovalCard from './components/chat/ApprovalCard.svelte';
import SignInCard from './components/SignInCard.svelte';
import ConsentCard from './components/ConsentCard.svelte';
import AccountRights from './components/AccountRights.svelte';
import WhatsNew from './components/WhatsNew.svelte';
import SettingsNav, { select, current } from './components/settings/SettingsNav.svelte';
import ProviderList from './components/providers/ProviderList.svelte';
import AgentCard from './components/providers/AgentCard.svelte';
import RoomMembers from './components/rooms/RoomMembers.svelte';
import RoomResources from './components/rooms/RoomResources.svelte';
import SettingsLink from './components/settings/SettingsLink.svelte';
import SettingsHint from './components/settings/SettingsHint.svelte';
import TelegramSettings from './components/settings/TelegramSettings.svelte';
import DiscordSettings from './components/settings/DiscordSettings.svelte';
import SlackSettings from './components/settings/SlackSettings.svelte';
import VaultSettings from './components/settings/VaultSettings.svelte';
import DrawerHead from './components/drawer/DrawerHead.svelte';
import DrawerProfile from './components/drawer/DrawerProfile.svelte';
import DrawerMemory from './components/drawer/DrawerMemory.svelte';
import DrawerGoals from './components/drawer/DrawerGoals.svelte';
import DrawerFiles from './components/drawer/DrawerFiles.svelte';
import BotList from './components/sidebar/BotList.svelte';
import NeedsBoard from './components/sidebar/NeedsBoard.svelte';
import UsageScreen from './components/usage/UsageScreen.svelte';
import RemindersTab from './components/routines/RemindersTab.svelte';
import TodosTab from './components/routines/TodosTab.svelte';
import HabitsTab from './components/routines/HabitsTab.svelte';
import Report from './components/report/Report.svelte';
import { bindReportEntry } from './lib/reportEntry.js';

import Button from './components/kit/Button.svelte';
import IconButton from './components/kit/IconButton.svelte';
import Field from './components/kit/Field.svelte';
import TextInput from './components/kit/TextInput.svelte';
import TextArea from './components/kit/TextArea.svelte';
import Select from './components/kit/Select.svelte';
import Switch from './components/kit/Switch.svelte';
import Checkbox from './components/kit/Checkbox.svelte';
import RadioGroup from './components/kit/RadioGroup.svelte';
import Slider from './components/kit/Slider.svelte';
import Modal from './components/kit/Modal.svelte';
import Section from './components/kit/Section.svelte';
import Row from './components/kit/Row.svelte';
import ListItem from './components/kit/ListItem.svelte';
import Avatar from './components/kit/Avatar.svelte';
import Tabs from './components/kit/Tabs.svelte';
import Chip from './components/kit/Chip.svelte';
import Badge from './components/kit/Badge.svelte';
import EmptyState from './components/kit/EmptyState.svelte';
import Spinner from './components/kit/Spinner.svelte';
import Skeleton from './components/kit/Skeleton.svelte';
import Tooltip from './components/kit/Tooltip.svelte';
import Toast from './components/kit/Toast.svelte';
import SplitView from './components/kit/SplitView.svelte';
import Toolbar from './components/kit/Toolbar.svelte';
import KitGallery from './components/kit/KitGallery.svelte';

import { iconSvg } from './lib/modelPicker.js';
import * as voiceChatLib from './lib/voiceChat.js';
import * as signInLib from './lib/signIn.js';
import * as toastLib from './lib/toast.js';
import { markdown, workspaceRef } from './lib/markdown.js';
import { turnDurations, durationText } from './lib/turnTime.js';
import { plainPreview } from './lib/preview.js';
import { offerEventsToShow } from './lib/pendingOffers.js';
import * as screenLayoutLib from './lib/screenLayout.js';
import ScreenControl from './components/ScreenControl.svelte';

window.DeloistUI = {
  version: 1,
  voiceChat: voiceChatLib,
  // Раскладка экрана бота (web/screen.js): ширина панели, углы островка чата, режимы.
  screenLayout: screenLayoutLib,
  mountScreenControl: (target, props) => mountComponent(ScreenControl, target, props),
  mountLimitRing: (target, props) => mountComponent(LimitRing, target, props),
  mountModelPicker: (target, props) => mountComponent(ModelPicker, target, props),
  // Модель в окне «Новый бот»: карточки на виду, остальные модели под «Ещё».
  mountNewBotModel: (target, props) => mountComponent(NewBotModel, target, props),
  mountChatModelPopover: (target, props) => mountComponent(ChatModelPopover, target, props),
  mountCatalog: (target, props) => mountComponent(Catalog, target, props),
  // Док ботов (web/dock.js): островок, лоток, меню и голос; плашка шагов у места бота; кнопка голоса отдельно.
  mountDock: (target, props) => mountComponent(Dock, target, props),
  mountStepPill: (target, props) => mountComponent(StepPill, target, props),
  mountMicButton: (target, props) => mountComponent(MicButton, target, props),
  // Меню кита по списку пунктов: «…» в шапке мини-чата дока.
  mountMenu: (target, props) => mountComponent(Menu, target, props),
  mountChatDates: (target, props) => mountComponent(ChatDates, target, props),
  mountMentionPopover: (target, props) => mountComponent(MentionPopover, target, props),
  mountWhatsNew: (target, props) => mountComponent(WhatsNew, target, props),
  mountKnowMe: (target, props) => mountComponent(KnowMe, target, props),
  mountVoiceChat: (target, props) => mountComponent(VoiceChat, target, props),
  // Чат: карточка «Нужно разрешение» и разметка ответов (абзацы, списки, таблицы, ссылки со значком).
  mountApprovalCard: (target, props) => mountComponent(ApprovalCard, target, props),
  markdown,
  workspaceRef,
  turnDurations,
  durationText,
  plainPreview,
  offerEventsToShow,
  mountSignInCard: (target, props) => mountComponent(SignInCard, target, props),
  // Отдельное согласие с документами Deloist после входа и на любой ответ сайта 451.
  mountConsentCard: (target, props) => mountComponent(ConsentCard, target, props),
  // Окно аккаунта: возврат денег, выгрузка данных, цены, удаление аккаунта.
  mountAccountRights: (target, props) => mountComponent(AccountRights, target, props),
  // Навигация окна настроек: выбранный раздел помнит сам модуль компонента.
  mountSettingsNav: (target, props) => mountComponent(SettingsNav, target, props),
  settingsNav: { select, current },
  // Строка-ссылка на подэкран и «?» с подсказкой в окне настроек.
  mountSettingsLink: (target, props) => mountComponent(SettingsLink, target, props),
  mountSettingsHint: (target, props) => mountComponent(SettingsHint, target, props),
  // Экран «Telegram»: токен, выбор отвечающего бота и привязка чата кодом.
  mountTelegramSettings: (target, props) => mountComponent(TelegramSettings, target, props),
  // Экран «Discord»: токен, выбор отвечающего бота и привязка личных сообщений кодом.
  mountDiscordSettings: (target, props) => mountComponent(DiscordSettings, target, props),
  // Экран «Slack»: два токена приложения, выбор отвечающего бота и привязка личных сообщений.
  mountSlackSettings: (target, props) => mountComponent(SlackSettings, target, props),
  // Экран «Менеджер паролей»: Bitwarden, 1Password и проверка входов для сайта.
  mountVaultSettings: (target, props) => mountComponent(VaultSettings, target, props),
  // Шапка панели бота (вкладки и «Закрыть»); содержимое вкладок рисует web/app.js.
  mountDrawerHead: (target, props) => mountComponent(DrawerHead, target, props),
  // Вкладка «Профиль» ящика бота: разделы кита; внешность и модель монтирует app.js.
  mountDrawerProfile: (target, props) => mountComponent(DrawerProfile, target, props),
  // Вкладки «Память», «Цели», «Файлы» ящика бота: данные и обработчики передаёт app.js.
  mountDrawerMemory: (target, props) => mountComponent(DrawerMemory, target, props),
  mountDrawerGoals: (target, props) => mountComponent(DrawerGoals, target, props),
  mountDrawerFiles: (target, props) => mountComponent(DrawerFiles, target, props),
  // Список слева: закреплённые, старшие, специалисты в запасе, группы.
  mountBotList: (target, props) => mountComponent(BotList, target, props),
  // Доска под «Нужен ты»: что ждёт тебя, что в работе, что ждёт в очереди и что не прочитано.
  mountNeedsBoard: (target, props) => mountComponent(NeedsBoard, target, props),
  // Окно «Использование и лимиты»: расход токенов, лимиты ботов, модели и подписки.
  mountUsageScreen: (target, props) => mountComponent(UsageScreen, target, props),
  // Окно «Рутины»: вкладки «Напоминания», «Задачи» и «Привычки».
  mountRemindersTab: (target, props) => mountComponent(RemindersTab, target, props),
  mountHabitsTab: (target, props) => mountComponent(HabitsTab, target, props),
  mountReport: (target, props) => mountComponent(Report, target, props),
  mountTodosTab: (target, props) => mountComponent(TodosTab, target, props),
  shouldShowSignIn: signInLib.shouldShowSignIn,
  // Окно «Подключить свою ИИ»: список слева (строки кита, логотипы как в выборе модели).
  ProviderList,
  // Карточка агента (Gemini, Claude, Codex…): вход, аккаунты, версия и обновление, модели, основной.
  mountAgentCard: (target, props) => mountComponent(AgentCard, target, props),
  // Состав группы в окне «Новая группа»: флажки кита по списку ботов.
  mountRoomMembers: (target, props) => mountComponent(RoomMembers, target, props),
  // Общие папки и навыки команды в том же окне.
  mountRoomResources: (target, props) => mountComponent(RoomResources, target, props),
  providerIconSvg: (provider) => iconSvg({ id: provider?.id, driver: provider?.driver }),

  // Базовые компоненты кита (ui/src/components/kit): их вставляют внутрь Svelte-экранов.
  Button,
  IconButton,
  Field,
  TextInput,
  TextArea,
  Select,
  Switch,
  Checkbox,
  RadioGroup,
  Slider,
  Modal,
  Section,
  Row,
  ListItem,
  Avatar,
  Tabs,
  Chip,
  Badge,
  EmptyState,
  Spinner,
  Skeleton,
  Tooltip,
  Toast,
  SplitView,
  Toolbar,
  // Витрина для /ui-demo.html: показывает все базовые компоненты в двух темах.
  KitGallery,
  // Контейнеры, которые старые экраны монтируют сами.
  mountModal: (target, props) => mountComponent(Modal, target, props),
  mountSection: (target, props) => mountComponent(Section, target, props),
  mountSplitView: (target, props) => mountComponent(SplitView, target, props),
  // Витрина и отдельные экраны без Svelte-обвязки монтируют базовые компоненты этим же помощником.
  mountComponent,
  toast: toastLib.toast,
  dismissToast: toastLib.dismissToast,
};

// Кнопка «Отчёт» в боковой панели главной страницы (если она есть на странице).
bindReportEntry(() => window.DeloistUI);
