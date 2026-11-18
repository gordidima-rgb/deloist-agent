// Что показывает аватар-шар за делом: знак занятия (кисть, сборка, поиск, документ, таблица, PDF, логотип
// приложения) и короткие эмоции от настоящих событий хода. Подключается во всех окнах, где есть аватары
// (программа, док, питомцы); сами окна только отдают сюда события и свои аватары.
//
//   toolLook(tool)              — чистая функция: какой знак у вызова инструмента ({ act, icon } или null);
//   moodFor(event)              — чистая функция: разовая эмоция по событию без истории ({ mood, seconds } или null);
//   stageLook(phase)            — чистая функция: фаза сцены оркестратора -> { state, act, mood? } из готовых видов шара;
//   waitOf(event)               — чистая функция: открылось или закрылось ожидание решения человека;
//   createTracker(avatarsOf)    — состояние окна: знак по ходу вызовов, ошибки подряд, редкая радость.
//
// Ничего не опрашивается: всё работает на событиях, которые окно и так получает.
(() => {
  // Какой знак у файла по расширению.
  const FILE_ACT = {
    pdf: 'pdf',
    xlsx: 'sheet', xls: 'sheet', xlsm: 'sheet', csv: 'sheet', tsv: 'sheet', ods: 'sheet',
    docx: 'doc', doc: 'doc', odt: 'doc', rtf: 'doc', md: 'doc', markdown: 'doc', txt: 'doc',
  };
  const DRAW_TOOLS = new Set(['generate_image', 'image', 'image_edit', 'edit_image', 'draw']);
  // Рисование в любом виде: навык imagegen, сервисы картинок, чтение/правка картинок (образцы и готовые кадры).
  const DRAW_TEXT = /image[-_ ]?gen|gpt-image|generated_images|openrouter-image|polza-image|xai-imagine|chatgpt-image|grok.?imagine/i;
  // Модель рисования по навыку, команде или аргументу model -> значок из web/icons/apps.
  const DRAW_MODELS = [
    [/chatgpt-image|gpt-image|dall-?e/i, 'openai'],
    [/gemini|imagen|nano.?banana/i, 'googlegemini'],
    [/xai-imagine|grok/i, 'x'],
  ];
  const IMAGE_EXT =new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg']);
  const SEARCH_TOOLS = new Set(['web_search', 'fetch', 'fetch_url', 'web_fetch']);
  // Сборка и компиляция: команды, которые строят программу.
  const BUILD = /\b(cargo\s+(build|check|test|clippy|run)|npm\s+(run\s+)?(build|test|ci|install|i)|pnpm\s+(build|install|test)|yarn\s+(build|install)|make|cmake|ninja|msbuild|dotnet\s+(build|publish|run|test)|gradlew?|mvn|tsc|vite\s+build|webpack|go\s+(build|test)|rustc|gcc|g\+\+|clang|javac|pip\s+install|pytest|vitest|jest)\b/i;
  // Слишком общие слова: это языки и инструменты командной строки, а не «приложение, в котором работает бот».
  const GENERIC = new Set([
    'git', 'go', 'x', 'npm', 'nodedotjs', 'python', 'rust', 'javascript', 'typescript', 'markdown', 'linux', 'line', 'vim', 'neovim',
    'react', 'svelte', 'vite', 'docker', 'kubernetes', 'latex', 'tailwindcss', 'nextdotjs', 'vuedotjs', 'numpy', 'pandas', 'pytorch',
    'tensorflow', 'jupyter', 'anaconda', 'homebrew', 'debian', 'ubuntu', 'terraform', 'ansible', 'nginx', 'redis', 'mongodb', 'mysql',
    'postgresql', 'sqlite', 'prometheus', 'grafana', 'jenkins', 'githubactions', 'swagger', 'wikipedia', 'stackoverflow', 'android',
    'apple', 'facebook', 'matrix', 'element', 'medium', 'messenger', 'safari', 'opera', 'brave', 'firefox', 'vivaldi', 'googlechrome',
  ]);

  let apps = null; // slug -> имя приложения (web/icons/apps/index.json)
  let names = null; // нормализованное имя или его первое слово -> slug
  const norm = (text) => String(text || '').toLowerCase().replace(/[^a-z0-9]/g, '');

  /** Список приложений со значками: slug -> имя. Без него знак «приложение» не показывается. */
  function setApps(index) {
    apps = index && typeof index === 'object' ? index : null;
    names = new Map();
    if (!apps) return;
    const first = new Map();
    for (const [slug, name] of Object.entries(apps)) {
      if (GENERIC.has(slug)) continue;
      names.set(slug, slug);
      names.set(norm(name), slug);
      const word = String(name || '').toLowerCase().split(/[^a-z0-9]+/)[0];
      if (word.length >= 4 && !GENERIC.has(word) && !(word in apps)) first.set(word, first.has(word) ? '' : slug); // «google» у многих: неоднозначное не берём
    }
    for (const [word, slug] of first) if (slug && !names.has(word)) names.set(word, slug);
  }

  let asked = false;
  /** Один раз берёт список значков с сервера (маленький файл); без него знак «приложение» не показывается. */
  function loadApps() {
    if (asked || typeof fetch !== 'function') return;
    asked = true;
    fetch('/icons/apps/index.json').then((res) => (res.ok ? res.json() : null)).then(setApps).catch(() => {});
  }

  const tokens = (text) => String(text || '').toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);

  function appIn(list) {
    if (!names) return '';
    for (const token of list) {
      if (token.length < 3) continue;
      const slug = names.get(token);
      if (slug) return slug;
    }
    return '';
  }

  // Названия, которыми люди зовут сервис, а значок лежит под другим именем.
  const ALIASES = { twitter: 'x' };
  /** Сервис из текста запроса поиска: любые строки аргументов и заголовок вызова. '' — не назван. */
  function serviceIn(tool, name) {
    if (!names) return '';
    const args = tool?.args && typeof tool.args === 'object' ? tool.args : {};
    const text = [name, tool?.title, ...Object.values(args).filter((value) => typeof value === 'string')].join(' ').slice(0, 400);
    for (const word of tokens(text)) {
      const alias = ALIASES[word];
      if (alias && apps && alias in apps) return alias;
      if (word.length < 3) continue;
      const slug = names.get(word);
      if (slug) return slug;
    }
    return '';
  }

  const extOf = (value) => {
    const match = /\.([a-z0-9]{2,8})(?:["'\s)]|$)/i.exec(String(value || ''));
    return match ? match[1].toLowerCase() : '';
  };

  const browserTool = (name) => /(?:^|[._\s])browser_(?:nav|page|mouse|keyboard|devtools|human|capture_key|logins|login)(?:$|\s)/i.test(String(name || ''));
  const SITE_DOMAINS = {
    'x.com': 'x', 'twitter.com': 'x', 'github.com': 'github', 'youtube.com': 'youtube', 'youtu.be': 'youtube',
    'telegram.org': 'telegram', 't.me': 'telegram', 'discord.com': 'discord', 'discord.gg': 'discord',
    'slack.com': 'slack', 'figma.com': 'figma', 'notion.so': 'notion', 'notion.com': 'notion',
    'reddit.com': 'reddit', 'instagram.com': 'instagram', 'tiktok.com': 'tiktok',
    'drive.google.com': 'googledrive', 'docs.google.com': 'googledocs',
  };
  const siteIcon = (url) => {
    try {
      const parsed = new URL(String(url));
      if (!['http:', 'https:'].includes(parsed.protocol)) return '';
      const host = parsed.hostname.toLowerCase();
      const domain = Object.keys(SITE_DOMAINS).find((item) => host === item || host.endsWith(`.${item}`));
      return domain ? `icons/apps/${SITE_DOMAINS[domain]}.svg` : '';
    } catch { return ''; }
  };

  /**
   * Знак занятия по вызову инструмента: `{ name, args, kind }` (kind — у внешних агентов: read, edit, execute, search, fetch…).
   * Возвращает `{ act, icon }`; для `act: 'app'` — ещё `icon` (адрес значка). null — знака нет, остаётся обычная загрузка.
   */
  function toolLook(tool) {
    const name = String(tool?.name || '').toLowerCase();
    const kind = String(tool?.kind || '').toLowerCase();
    const args = tool?.args && typeof tool.args === 'object' ? tool.args : {};
    const command = String(args.command || '');
    const path = String(args.path || args.file || args.filename || '');

    const title = String(tool?.title || '');
    if (DRAW_TOOLS.has(name) || /(^|_)image($|_)/.test(name) || kind === 'generate' || DRAW_TEXT.test(name) || DRAW_TEXT.test(command) || DRAW_TEXT.test(title)) {
      // Через какую модель рисует: её значок на шаре чередуется со знаком рисования.
      const model = DRAW_MODELS.find(([re]) => re.test(`${name} ${command} ${title} ${args.model || ''}`));
      return model ? { act: 'draw', icon: `icons/apps/${model[1]}.svg` } : { act: 'draw' };
    }
    // Просмотр или правка картинки — тоже рисование (но не браузерные инструменты: там знак сайта или поиска).
    if (!browserTool(name) && (IMAGE_EXT.has(extOf(path)) || (kind === 'read' && IMAGE_EXT.has(extOf(tool?.title))))) return { act: 'draw' };
    const icon = siteIcon(args.url);
    if (icon && (browserTool(name) || kind === 'fetch')) return { act: 'app', icon };
    if (browserTool(name) || SEARCH_TOOLS.has(name) || kind === 'search' || kind === 'fetch') {
      // Поиск без адреса: если запрос называет сервис («OpenBot nightly-labs github»), на шаре его значок, а не общая лупа.
      if (!browserTool(name)) {
        const slug = serviceIn(tool, name);
        if (slug) return { act: 'app', icon: `icons/apps/${slug}.svg` };
      }
      return { act: 'search' };
    }
    const file = FILE_ACT[extOf(path)] || FILE_ACT[extOf(command)];
    if (file) return { act: file };
    // Приложение называет сам вызов: имя инструмента (telegram_posts, mcp__godot__…), заголовок агента, первое слово команды.
    const slug = appIn([...tokens(name), ...tokens(command).slice(0, 1)]);
    if (slug) return { act: 'app', icon: `icons/apps/${slug}.svg` };
    if (BUILD.test(command)) return { act: 'build' };
    return null;
  }

  // ------------------------------------------------------------ Настроения

  const RATE = /\b429\b|rate.?limit|too many|usage limit|quota|лимит/i;
  const OVERLOADED = /overloaded|\b5\d\d\b|bad gateway|перегружен/i;
  const UNAVAILABLE = /model_unavailable|unavailable|consent|недоступн|согласи/i;

  /** Что значит текст ошибки хода: sleep (лимит), dizzy (перегрузка), sad (недоступно, нет согласия), error (остальное). */
  function errorMood(text) {
    const raw = String(text || '');
    if (RATE.test(raw)) return { mood: 'sleep', seconds: 5 };
    if (OVERLOADED.test(raw)) return { mood: 'dizzy', seconds: 4 };
    if (UNAVAILABLE.test(raw)) return { mood: 'sad', seconds: 5 };
    return { mood: 'error', seconds: 5 };
  }

  /** Разовая эмоция по событию, которой не нужна история. */
  function moodFor(event) {
    switch (event?.type) {
      case 'approval_closed':
        return event.status === 'denied' ? { mood: 'sad', seconds: 2 } : null;
      case 'model_retry':
      case 'room_retry':
        return { mood: 'grumpy', seconds: 1.5 };
      case 'model_switched':
        return { mood: 'surprise', seconds: 1.2 };
      case 'compacting':
        return { mood: 'dizzy', seconds: 3 };
      case 'memory_learned':
        return { mood: 'love', seconds: 1.8 };
      case 'skill_learned':
        return { mood: 'star', seconds: 1.8 };
      default:
        return null;
    }
  }

  const WAIT_OPEN = new Set(['approval', 'question']);
  const WAIT_CLOSE = new Set(['approval_closed', 'question_closed']);

  /**
   * Ожидание решения человека: одобрение, вопрос, просьба посмотреть браузер, предложение специалиста.
   * `{ open: id }` — открылось, `{ close: id }` — закрылось, null — к ожиданию не относится.
   * У просьбы браузера и предложения нет события закрытия: они снимаются следующим шагом бота (см. `clears`).
   */
  function waitOf(event) {
    if (!event) return null;
    if (WAIT_OPEN.has(event.type) && event.id) return { open: event.id };
    if (WAIT_CLOSE.has(event.type) && event.id) return { close: event.id };
    if (event.type === 'browser_needs_human') return { open: 'human' };
    if (event.type === 'specialist_offer' && event.offer?.id) return { open: `offer:${event.offer.id}` };
    if (event.type === 'team_proposal' && event.proposal?.id) return { open: `offer:${event.proposal.id}` };
    return null;
  }

  /** События, после которых бот снова работает сам: просьбы без события закрытия (браузер, предложение) снимаются. */
  const clears = (event) => ['tool_start', 'tool_update', 'delegate_start', 'turn_start', 'turn_end'].includes(event?.type)
    || (event?.type === 'activity' && event.call?.status === 'running');

  // -------------------------------------------------------------- Трекер

  const JOY_GAP = 2000; // радость от удачного результата не чаще раза в 2 секунды
  const STREAK = 3; // столько неудач подряд — головокружение

  /**
   * Состояние одного окна. `avatarsOf(botId)` — живые аватары этого бота в окне (массив).
   * `feed(event, now)` ведёт знак по вызовам и эмоции по результатам.
   */
  function createTracker(avatarsOf, clock = () => performance.now()) {
    loadApps();
    const calls = new Map(); // botId -> Map(callId -> { look, browser }) в порядке начала
    const streak = new Map(); // botId -> неудач подряд
    const lastJoy = new Map(); // botId -> когда последний раз радовался
    const failed = new Map(); // botId -> чем кончился последний ход: true — неудачей
    const reason = new Map(); // botId -> текст последней ошибки хода
    const sites = new Map(); // последний явно открытый сайт в текущем ходе
    const navigations = new Map(); // botId -> Set(callId), чтобы сбросить сайт при ошибке перехода

    const each = (id, fn) => { for (const avatar of avatarsOf(id) || []) if (avatar) fn(avatar); };
    const feel = (id, spec) => { if (spec) each(id, (avatar) => avatar.feel?.(spec.mood, spec.seconds)); };

    function show(id) {
      // Знак того вызова, что идёт сейчас: последнего из начатых и не законченных.
      const pending = calls.get(id);
      let look = null;
      if (pending) for (const entry of pending.values()) {
        const icon = entry.browser && sites.get(id);
        look = entry.browser ? (icon ? { act: 'app', icon } : { act: 'search' }) : entry.look;
      }
      each(id, (avatar) => avatar.set?.({ act: look?.act || '', icon: look?.icon || '' }));
    }

    function begin(id, callId, look, browser) {
      if (!calls.has(id)) calls.set(id, new Map());
      calls.get(id).set(callId, { look, browser });
      show(id);
    }

    function end(id, callId, status) {
      if (navigations.get(id)?.delete(callId) && status !== 'ok') sites.delete(id);
      calls.get(id)?.delete(callId);
      show(id);
      if (status === 'error' || status === 'timeout') {
        const n = (streak.get(id) || 0) + 1;
        if (n >= STREAK) {
          streak.set(id, 0);
          feel(id, { mood: 'dizzy', seconds: 3 });
        } else {
          streak.set(id, n);
          feel(id, { mood: 'grumpy', seconds: 1.5 });
        }
      } else if (status === 'ok') {
        streak.set(id, 0);
        const now = clock();
        if (now - (lastJoy.get(id) || -Infinity) >= JOY_GAP) {
          lastJoy.set(id, now);
          feel(id, { mood: 'joy', seconds: 0.5 });
        }
      }
    }

    function reset(id) {
      calls.delete(id);
      sites.delete(id);
      navigations.delete(id);
      show(id);
    }

    function lookFor(id, tool, callId) {
      const name = String(tool.name || '');
      const args = tool.args || {};
      if (browserTool(name)) {
        if (/browser_nav(?:$|\s)/i.test(name) && args.url && callId) {
          if (!navigations.has(id)) navigations.set(id, new Set());
          navigations.get(id).add(callId);
        }
        if (args.url) sites.set(id, siteIcon(args.url));
        else if (/browser_nav(?:$|\s)/i.test(name) && ['back', 'forward', 'tabs', 'switch', 'close'].includes(args.action)) sites.delete(id);
      }
      return toolLook(tool);
    }

    return {
      sync: show,
      /** Чем кончился последний ход этого бота: true — неудачей. */
      failedLast: (id) => failed.get(id) === true,
      /** Настроение для конца хода по последней ошибке. */
      endMood(id) {
        const text = reason.get(id);
        reason.delete(id);
        return errorMood(text);
      },
      feed(event) {
        const id = event?.botId;
        if (!id) return;
        switch (event.type) {
          case 'turn_start':
            reset(id);
            streak.set(id, 0);
            reason.delete(id);
            break;
          case 'turn_end':
            failed.set(id, Boolean(event.failed));
            reset(id);
            break;
          case 'tool_start':
            if (event.callId) begin(id, event.callId, lookFor(id, { name: event.name, args: event.args }, event.callId), browserTool(event.name));
            break;
          case 'tool_update':
            if (event.callId) end(id, event.callId, event.status);
            break;
          case 'activity': {
            // Внешний агент присылает одну строку на вызов и обновляет её: считаем переходы, а не повторы.
            const call = event.call || {};
            if (!call.id) break;
            const known = calls.get(id)?.has(call.id);
            if (call.status === 'running' && !known) {
              begin(id, call.id, lookFor(id, call, call.id), browserTool(call.name));
            } else if (call.status === 'running' && known) {
              const site = sites.get(id);
              const look = lookFor(id, call, call.id);
              const next = { look, browser: browserTool(call.name) };
              if (site !== sites.get(id) || JSON.stringify(calls.get(id).get(call.id)) !== JSON.stringify(next)) {
                calls.get(id).set(call.id, next);
                show(id);
              }
            } else if ((call.status === 'ok' || call.status === 'error') && known) {
              end(id, call.id, call.status);
            }
            break;
          }
          case 'error':
            reason.set(id, event.message);
            break;
          case 'delegate_end': {
            // Ушёл ли получатель с неудачей, видно по концу его хода.
            feel(id, failed.get(event.to) ? { mood: 'grumpy', seconds: 1.5 } : { mood: 'joy', seconds: 1 });
            break;
          }
          default:
            feel(id, moodFor(event));
            break;
        }
      },
    };
  }

  // Сцена оркестратора: фаза расспроса -> готовый вид шара (своих настроений не заводим).
  // state и act — как у обычной работы (значок занятия), mood — разовая эмоция на входе в фазу.
  const STAGE_LOOKS = {
    listening: { state: 'idle', act: '' },
    thinking: { state: 'work', act: 'search' },
    typing: { state: 'work', act: 'doc' },
    asking: { state: 'wait', act: '', mood: 'surprise' },
    done: { state: 'idle', act: '', mood: 'star' },
  };
  function stageLook(phase) {
    return Object.prototype.hasOwnProperty.call(STAGE_LOOKS, phase) ? { ...STAGE_LOOKS[phase] } : null;
  }

  window.CrewAvatarStates = { toolLook, moodFor, waitOf, clears, errorMood, stageLook, setApps, createTracker };
})();
