// Логика выбора модели без DOM: из ответа /api/providers и настроек собирает группы
// «провайдер → модели», фильтрует по поиску и превращает выбор в пару { driver, model }.
// Те же два поля бота и общих настроек, что раньше вводили руками: старые боты читаются без миграции.
import { tr } from './i18n.js';
import { PROVIDER_ICONS } from './providerIcons.js';

/// Драйверы без списка моделей: человек вписывает имя сам (ключ API, Anthropic, Gemini, программа).
const MANUAL = [
  { driver: 'api', ready: (s) => s.hasKey, typed: true },
  { driver: 'anthropic', ready: (s) => s.hasAnthropicKey, typed: true },
  { driver: 'gemini', ready: (s) => s.hasGeminiKey, typed: true },
  { driver: 'cli', ready: (s) => Boolean(s.cliCommand), typed: true },
  { driver: 'acp', ready: (s) => Boolean(s.acpCommand), typed: false },
];

const isFree = (m) => m.free === true || /:free$/i.test(String(m.id || ''));

function visibleModels(p) {
  const hidden = new Set(p.hiddenModels || []);
  return (p.models || []).filter((m) => !hidden.has(m.id));
}

function nameOfDriver(driver) {
  if (driver === 'deloist') return tr('Подписка Deloist');
  if (driver === 'demo') return tr('Демо, без ключа');
  if (driver === 'api') return tr('Модель по ключу API');
  if (driver === 'anthropic') return 'Anthropic (Claude)';
  if (driver === 'gemini') return 'Google Gemini';
  if (driver === 'cli') return tr('Программа на компьютере');
  if (driver === 'acp') return tr('Агент ACP');
  const at = driver.indexOf(':');
  return at > 0 ? driver.slice(at + 1) : driver;
}

/// Группы для списка. `value` — текущий выбор { driver, model }; его провайдер показываем всегда,
/// даже если он выключен или удалён: старый бот должен видеть, на чём он работает.
export function buildGroups({ providers = [], plan = null, settings = {}, value = { driver: '', model: '' } } = {}) {
  const groups = [];
  const seen = new Set();
  const push = (g) => {
    seen.add(g.driver);
    groups.push(g);
  };

  if (plan && Array.isArray(plan.models)) {
    push({
      id: 'deloist',
      driver: 'deloist',
      name: tr('Подписка Deloist'),
      kind: 'plan',
      models: plan.models.map((m) => ({ id: m.id, name: m.name || m.id, badge: 'sub' })),
      defaultModel: plan.defaultModel || '',
    });
  }

  for (const p of providers) {
    const usable = p.enabled !== false && p.status === 'connected';
    if (!usable || !p.driver) continue;
    push({
      id: p.id,
      driver: p.driver,
      name: p.name || p.id,
      kind: p.kind || 'key',
      icon: p.kind === 'acp' && p.icon ? p.id : '',
      models: visibleModels(p).slice().sort((a, b) => Number(isFree(b)) - Number(isFree(a)))
        .map((m) => (m.id === 'default' ? { id: m.id, name: tr('По умолчанию — выбирает {agent}', { agent: p.name || p.id }), badge: 'default' } : { id: m.id, name: m.name || m.id, badge: isFree(m) ? 'free' : '' }))
        .sort((a, b) => Number(b.badge === 'default') - Number(a.badge === 'default')),
    });
  }

  // Демо нужно, только когда больше ничего не подключено (или бот уже на нём).
  const manualReady = MANUAL.some((m) => m.ready(settings) || value.driver === m.driver);
  if ((groups.length === 0 && !manualReady) || value.driver === 'demo') push({ id: 'demo', driver: 'demo', name: tr('Демо, без ключа'), kind: 'demo', models: [{ id: '', name: tr('Ответы по сценарию'), badge: '' }] });

  for (const m of MANUAL) {
    if (!m.ready(settings) && value.driver !== m.driver) continue;
    // Модель этих драйверов вписывают руками (поиск → «Использовать …») или берут из общих настроек.
    push({ id: m.driver, driver: m.driver, name: nameOfDriver(m.driver), kind: 'manual', manual: m.typed, models: [{ id: '', name: tr('Модель из настроек'), badge: '' }] });
  }

  // Текущий провайдер пропал из списка (выключен, удалён): добавляем его, чтобы выбор был виден.
  if (value.driver && !seen.has(value.driver)) {
    push({ id: value.driver, driver: value.driver, name: nameOfDriver(value.driver), kind: 'missing', models: [] });
  }

  // Текущая модель, которой нет в списке (вписана руками или скрыта), остаётся отдельной строкой.
  const current = groups.find((g) => g.driver === value.driver);
  if (current && value.model && !current.models.some((m) => m.id === value.model) && current.kind !== 'plan' && current.kind !== 'demo') {
    current.models = [{ id: value.model, name: value.model, badge: '', extra: true }, ...current.models];
  }

  for (const g of groups) {
    // Провайдер без списка моделей работает на своей модели по умолчанию.
    if (g.kind !== 'manual' && g.kind !== 'demo' && (g.models.length === 0 || (value.driver === g.driver && !value.model && g.kind !== 'plan'))) {
      g.models = [{ id: '', name: tr('Модель по умолчанию'), badge: '' }, ...g.models];
    }
  }
  return groups;
}

const norm = (s) => String(s || '').toLowerCase();

/// Поиск по названию провайдера, названию модели и её id. Пустые группы выпадают.
/// В группах «своя модель» непустой запрос превращается в строку «Использовать …».
export function filterGroups(groups, query, only = '') {
  const q = norm(query).trim();
  const out = [];
  for (const g of groups) {
    if (only && g.id !== only) continue;
    const groupHit = q && norm(g.name).includes(q);
    let models = g.models;
    if (q && !groupHit) models = models.filter((m) => norm(`${m.name} ${m.id}`).includes(q));
    if (g.manual && query.trim()) {
      const typed = query.trim();
      if (!models.some((m) => m.id === typed)) models = [...models, { id: typed, name: typed, badge: '', typed: true }];
    }
    if (models.length || (g.manual && !q)) out.push({ ...g, models });
  }
  return out;
}

/// Выбор строки → то, что уходит в поля бота.
export function toValue(group, model) {
  return { driver: group.driver, model: model.id };
}

/// Подписка берёт модель из своего списка; всё остальное (пусто, чужое имя) значит модель по умолчанию.
const planModel = (group, model) => (group.models.some((m) => m.id === model) ? model : group.defaultModel);

/// Строка совпадает с текущим выбором. У подписки пустая модель значит «модель по умолчанию».
export function isCurrent(group, model, value) {
  if (group.driver !== value.driver) return false;
  if (group.kind === 'plan') return planModel(group, value.model) === model.id;
  // Демо отвечает по сценарию и модель не смотрит: любая сохранённая модель значит ту же единственную строку.
  if (group.kind === 'demo') return model.id === '';
  return (value.model || '') === model.id;
}

/// Подпись выбора в строке «Модель»: «Провайдер/Модель» или «как в настройках».
export function labelOf(groups, value) {
  if (!value.driver) return { text: tr('как в настройках'), group: null };
  const group = groups.find((g) => g.driver === value.driver);
  const provider = group?.name || nameOfDriver(value.driver);
  const modelId = group?.kind === 'plan' ? planModel(group, value.model) : value.model;
  if (group?.kind === 'demo') return { text: provider, group };
  const model = group?.models.find((m) => m.id === modelId);
  const modelName = model?.name || modelId;
  return { text: modelName && model?.id !== '' ? `${provider}/${modelName}` : provider, group };
}

// ---------------------------------------------------------------------------
// Логотипы провайдеров
// ---------------------------------------------------------------------------

/// Свои значки: Deloist, демо и программа. Строки зашиты в сборку, пользовательских данных тут нет.
const OWN_ICONS = {
  deloist: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="3.5" width="17" height="17" rx="5" /><path d="m8.5 12.4 2.4 2.4 4.6-5" /></svg>',
  demo: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5v4M12 16.5v4M3.5 12h4M16.5 12h4" /><path d="m12 8.5 1.2 2.3 2.3 1.2-2.3 1.2L12 15.5l-1.2-2.3L8.5 12l2.3-1.2z" /></svg>',
  cli: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="3.5" /><path d="m8 10 3 2.5L8 15M13 15h3.5" /></svg>',
};

/// Точные идентификаторы → ключ значка.
const ICON_EXACT = {
  api: 'openai',
  openai: 'openai',
  chatgpt: 'openai',
  anthropic: 'claude',
  google: 'gemini',
  zai: 'zai',
  zhipu: 'zhipu',
  alice: 'yandex',
  yandex: 'yandex',
  amp: 'amp',
  goose: 'goose',
  cline: 'cline',
  qoder: 'qoder',
  xai: 'xai',
  grok: 'grok',
  mistral: 'mistral',
  perplexity: 'perplexity',
  huggingface: 'huggingface',
  together: 'together',
  fireworks: 'fireworks',
  cohere: 'cohere',
  moonshot: 'moonshot',
};

/// Подстроки (для агентов ACP: `claude-code`, `gemini-cli`, `github-copilot` …); порядок важен.
const ICON_PART = [
  ['claude', 'claude'],
  ['codex', 'openai'], // codex-acp is shown as «ChatGPT»: the OpenAI mark, as in the providers window
  ['gemini', 'gemini'],
  ['opencode', 'opencode'],
  ['cursor', 'cursor'],
  ['copilot', 'githubcopilot'],
  ['cline', 'cline'],
  ['kilo', 'kilocode'],
  ['goose', 'goose'],
  ['qoder', 'qoder'],
  ['qwen', 'qwen'],
  ['kimi', 'kimi'],
  ['deepseek', 'deepseek'],
  ['groq', 'groq'],
  ['nvidia', 'nvidia'],
  ['openrouter', 'openrouter'],
  ['ollama', 'ollama'],
  ['cerebras', 'cerebras'],
  ['lmstudio', 'lmstudio'],
];

/// Ключ значка группы: `deloist`, `demo`, `cli` или ключ из PROVIDER_ICONS; неизвестная группа — null.
export function iconFor(group) {
  if (!group) return null;
  const driver = String(group.driver || '');
  const tail = driver.includes(':') ? driver.slice(driver.indexOf(':') + 1) : driver;
  for (const raw of [group.id, tail, driver]) {
    const id = String(raw || '').toLowerCase();
    if (!id) continue;
    if (id === 'deloist' || id === 'demo' || id === 'cli') return id;
    if (ICON_EXACT[id]) return ICON_EXACT[id];
    for (const [part, key] of ICON_PART) if (id.includes(part)) return key;
  }
  return null;
}

/// Строка SVG для значка группы (или null). Только зашитые константы.
export function iconSvg(group) {
  const key = iconFor(group);
  return key ? OWN_ICONS[key] || PROVIDER_ICONS[key] || null : null;
}

/// Вторая строка в шапке панели: что за провайдер.
export function subtitleOf(group) {
  if (group.kind === 'plan') return tr('Подписка');
  if (group.kind === 'demo') return tr('Без ключа');
  if (group.kind === 'acp') return tr('Агент ACP');
  if (group.kind === 'manual') return tr('Своя модель');
  if (group.kind === 'missing') return tr('Не подключено');
  if (group.kind === 'key') {
    const real = group.models.filter((m) => m.id);
    return real.length && real.every((m) => m.badge === 'free') ? tr('бесплатно') : tr('по ключу API');
  }
  return '';
}

/// Сколько моделей показываем сразу; остальные и «старые» (extra) уходят под «Ещё модели».
export const VISIBLE_MODELS = 8;

/// Делит модели группы на видимые и свёрнутые: лишние сверх первых восьми. Свою и вписанную модель не прячем.
export function splitModels(group) {
  const main = [];
  const more = [];
  group.models.forEach((m, i) => (m.typed || m.extra || i < VISIBLE_MODELS ? main : more).push(m));
  return { main, more };
}
