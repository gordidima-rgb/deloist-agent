// Подписи шагов инструментов для чата: русская фраза для каждого известного инструмента
// (английский — в web/i18n.js), имя без префикса MCP, «над чем» работает шаг, признак служебного
// инструмента, текст живого шага и итог хода словами. Подключается перед app.js.
(() => {
  const getName = (call) => (typeof call === 'string' ? call : (call && call.name) || '');

  // Убираем любой префикс mcp__<server>__ или mcp.<server>. — без списка имён.
  const plainName = (name) => String(name || '').replace(/^mcp(?:__|\.)[a-z0-9_-]+?(?:__|\.)/i, '');

  const LABELS = {
    add_todo: 'Добавляет задачу',
    archive_habit: 'Архивирует привычку',
    ask_user: 'Спрашивает человека',
    assign_task: 'Даёт задачу участнику',
    await_event: 'Ждёт событие',
    browser_capture_key: 'Сохраняет ключ',
    browser_devtools: 'Смотрит в отладчик',
    browser_human: 'Зовёт тебя в браузер',
    browser_keyboard: 'Печатает',
    browser_login: 'Входит на сайт',
    browser_logins: 'Смотрит сохранённые входы',
    browser_mouse: 'Двигает мышью',
    browser_nav: 'Открывает страницу',
    browser_page: 'Читает страницу',
    browser_show_to_human: 'Просит посмотреть на экран',
    check_habit: 'Отмечает привычку',
    complete_goal: 'Завершает цель',
    complete_todo: 'Выполняет задачу',
    create_bot: 'Создаёт бота',
    create_goal: 'Ставит цель',
    create_habit: 'Заводит привычку',
    create_routine: 'Создаёт рутину',
    create_workflow: 'Создаёт процесс',
    delete_goal: 'Удаляет цель',
    delete_routine: 'Удаляет рутину',
    delete_todo: 'Удаляет задачу',
    delete_workflow: 'Удаляет процесс',
    edit_workflow: 'Правит процесс',
    finish_turn: 'Подводит итог',
    generate_image: 'Рисует картинку',
    list_bots: 'Смотрит команду',
    list_files: 'Смотрит файлы',
    list_goals: 'Смотрит цели',
    list_habits: 'Смотрит привычки',
    list_routines: 'Смотрит рутины',
    list_todos: 'Смотрит задачи',
    list_workflows: 'Смотрит процессы',
    offer_specialist: 'Предлагает специалиста',
    pause_goal: 'Ставит цель на паузу',
    propose_bot_change: 'Предлагает поправить участника',
    propose_routine: 'Предлагает расписание',
    propose_rule: 'Предлагает правило',
    propose_team: 'Предлагает команду',
    read_file: 'Читает файл',
    read_help: 'Читает справку',
    read_history: 'Читает прошлый разговор',
    read_skill: 'Читает навык',
    read_workflow: 'Читает процесс',
    read_workflow_runs: 'Смотрит запуски процесса',
    recall: 'Ищет в памяти',
    remember: 'Запоминает',
    report_goal_progress: 'Записывает прогресс цели',
    resume_goal: 'Снимает цель с паузы',
    run_command: 'Запускает команду',
    run_goal: 'Запускает цель',
    run_routine: 'Запускает рутину',
    run_script: 'Запускает скрипт',
    run_workflow: 'Запускает процесс',
    save_script: 'Сохраняет скрипт',
    save_skill: 'Сохраняет навык',
    search_catalog: 'Ищет в каталоге',
    search_history: 'Ищет в прошлых разговорах',
    share_file: 'Передаёт файл',
    stop_workflow_run: 'Останавливает запуск процесса',
    task_status: 'Смотрит задачи',
    team_memory_merge: 'Сливает факты памяти',
    team_memory_report: 'Пишет итог разбора памяти',
    team_memory_review: 'Разбирает память команды',
    team_status: 'Смотрит команду',
    telegram_posts: 'Читает посты Telegram',
    update_bot: 'Настраивает бота',
    update_goal: 'Меняет цель',
    update_habit: 'Меняет привычку',
    update_routine: 'Меняет рутину',
    update_todo: 'Меняет задачу',
    web_fetch: 'Открывает адрес',
    web_search: 'Ищет в интернете',
    workspace_graph: 'Строит граф рабочей папки',
    write_file: 'Записывает файл',
  };

  const STEP_KIND_SOURCES = {
    execute: 'Запускает команду',
    edit: 'Записывает файл',
    read: 'Читает файл',
    search: 'Ищет',
    fetch: 'Открывает адрес',
    think: 'Думает',
    delete: 'Удаляет файл',
    move: 'Переносит файл',
  };

  const MESSY_TITLE = /^[\s{["]|[{}`\r\n]|":/;

  const tr = (text) => (window.tr ? window.tr(text) : text);

  const fallback = (name) => {
    const words = String(name || '').replace(/[_-]+/g, ' ').trim();
    if (!words) return '';
    return words.charAt(0).toUpperCase() + words.slice(1);
  };

  function agentLabel(call) {
    const name = String(call?.name || '').trim();
    const kind = STEP_KIND_SOURCES[call?.kind];
    const kindLabel = kind ? tr(kind) : '';
    if (kindLabel && (MESSY_TITLE.test(name) || /^(execute|write|edit|read|delete|move|fetch)\b/i.test(name))) return kindLabel;
    if (!name || name.length > 60 || MESSY_TITLE.test(name)) return kindLabel || tr('Выполняет шаг');
    return name;
  }

  function label(call) {
    const c = typeof call === 'string' ? { name: call } : (call || {});
    const name = getName(c);
    const plain = plainName(name);
    if (LABELS[plain]) return tr(LABELS[plain]);
    if (LABELS[name]) return tr(LABELS[name]);
    const candidate = plain || name;
    if (/^[A-Za-z0-9_.-]+$/.test(candidate) && (candidate.indexOf('_') >= 0 || candidate.indexOf(' ') < 0)) {
      return tr(fallback(candidate));
    }
    return agentLabel(c);
  }

  const clip = (value, n = 60) => {
    const s = String(value ?? '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };
  const hostOf = (url) => {
    try {
      return new URL(/^[a-z][a-z0-9+.-]*:/i.test(url) ? url : 'https://' + url).hostname.replace(/^www\./, '');
    } catch {
      return String(url || '').slice(0, 40);
    }
  };
  const maskValue = (value) => {
    let s = String(value ?? '');
    s = s.replace(/Bearer\s+[^"\s]+/gi, '•••');
    s = s.replace(/sk-[A-Za-z0-9_-]+/gi, '•••');
    s = s.replace(/ghp_[A-Za-z0-9]+/g, '•••');
    s = s.replace(/\b[0-9a-f]{32,}\b/gi, '•••');
    // Похоже на ключ: длинный кусок base64 с цифрами и буквами обоих регистров; путь вроде «/home/user/projects» — нет.
    s = s.replace(/[A-Za-z0-9+/]{32,}={0,2}/g, (m) => (/\d/.test(m) && /[a-z]/.test(m) && /[A-Z]/.test(m) ? '•••' : m));
    s = s.replace(/([?&]token=)[^&#\s"]+/gi, '$1•••');
    return s;
  };
  function target(call) {
    const c = typeof call === 'string' ? { name: call } : (call || {});
    const args = (c.args && typeof c.args === 'object') ? c.args : {};
    const plain = plainName(getName(c));
    const str = (v) => (typeof v === 'string' ? v : (typeof v === 'number' ? String(v) : ''));
    const isSecretKey = (k) => /password|token|secret|key/i.test(String(k || ''));
    if (plain === 'browser_nav' || plain === 'web_fetch') {
      return args.url ? hostOf(str(args.url)) : '';
    }
    if (plain === 'web_search') {
      const v = str(args.query) || str(args.q);
      return v ? clip(maskValue(v)) : '';
    }
    if (plain === 'browser_mouse') {
      return clip(str(args.label) || str(args.selector) || (args.ref ? '#' + str(args.ref) : ''));
    }
    if (plain === 'browser_keyboard') {
      const typed = str(args.text);
      if (typed) {
        const secret = /passw|парол|secret|token|cvv|card|карт/i.test(str(args.label) + ' ' + str(args.selector));
        return secret ? '••••••' : '«' + clip(typed, 40) + '»';
      }
      return clip(str(args.keys));
    }
    const ordered = ['command', 'path', 'file_path', 'url', 'query', 'q', 'pattern', 'bot', 'to', 'name', 'room', 'task', 'description', 'text'];
    for (const key of ordered) {
      if (!Object.prototype.hasOwnProperty.call(args, key)) continue;
      if (isSecretKey(key)) continue;
      const raw = args[key];
      const value = str(raw);
      if (!value.trim()) continue;
      let out = maskValue(value.trim());
      out = out.replace(/\s+/g, ' ');
      if (out.length > 120) out = out.slice(0, 119) + '…';
      return out;
    }
    return '';
  }

  function hidden(call) {
    const name = getName(call);
    const base = plainName(name).toLowerCase();
    const compact = base.replace(/[\s_-]/g, '');
    return compact === 'toolsearch' || base === 'tool_search' || base === 'toolsearch';
  }

  function liveText(call, label, target) {
    const isFile = /^(read_file|write_file|list_files)$/.test(plainName(getName(call))) || (call && call.kind === 'edit') || (call && call.kind === 'read');
    const lbl = label == null ? '' : String(label);
    const tgt = target == null ? '' : String(target);
    if (!tgt || (call && call.kind === 'execute') || plainName(getName(call)) === 'run_command') return lbl;
    const shown = (isFile ? tgt.split(/[\\/]/).pop() : tgt).slice(0, 48);
    return !shown || lbl.includes(shown) ? lbl : lbl + ' ' + shown;
  }

  // Что сделал ход, словами: «прочитал 2 файла, искал 3 раза». Мысли (`think`) и скрытые шаги не считаем.
  const SUMMARY = {
    read: ['прочитал {n} файл', 'прочитал {n} файла', 'прочитал {n} файлов'],
    search: ['искал {n} раз', 'искал {n} раза', 'искал {n} раз'],
    web: ['открыл {n} страницу', 'открыл {n} страницы', 'открыл {n} страниц'],
    write: ['записал {n} файл', 'записал {n} файла', 'записал {n} файлов'],
    run: ['запустил {n} команду', 'запустил {n} команды', 'запустил {n} команд'],
    task: ['передал {n} задачу', 'передал {n} задачи', 'передал {n} задач'],
    other: ['ещё {n} шаг', 'ещё {n} шага', 'ещё {n} шагов'],
  };
  const SUMMARY_SHOWN = 3;

  function kindOf(call) {
    const plain = plainName(getName(call));
    const kind = call && call.kind;
    if (kind === 'think') return '';
    if (plain === 'read_file' || plain === 'list_files' || kind === 'read') return 'read';
    if (/^(web_search|search_|recall)/.test(plain) || kind === 'search') return 'search';
    if (plain === 'web_fetch' || plain.startsWith('browser_') || kind === 'fetch') return 'web';
    if (plain === 'write_file' || kind === 'edit' || kind === 'delete' || kind === 'move') return 'write';
    if (plain === 'run_command' || plain === 'run_script' || kind === 'execute') return 'run';
    if (plain === 'assign_task') return 'task';
    return 'other';
  }

  // `plural(n, one, few, many)` приходит из app.js: там правила числа для обоих языков.
  const phrase = (kind, n, plural) => plural(n, ...SUMMARY[kind]).replace('{n}', n);
  const more = (n, plural) => phrase('other', n, plural);

  function summary(calls, plural) {
    const counts = {};
    for (const call of calls) {
      const kind = hidden(call) ? '' : kindOf(call);
      if (kind) counts[kind] = (counts[kind] || 0) + 1;
    }
    const named = Object.keys(SUMMARY).filter((k) => k !== 'other' && counts[k]);
    const shown = [...named].sort((a, b) => counts[b] - counts[a]).slice(0, SUMMARY_SHOWN);
    const rest = (counts.other || 0) + named.filter((k) => !shown.includes(k)).reduce((sum, k) => sum + counts[k], 0);
    const parts = named.filter((k) => shown.includes(k)).map((k) => phrase(k, counts[k], plural));
    if (rest) parts.push(more(rest, plural));
    return parts.join(', ');
  }

  window.CrewToolLabels = { label, target, hidden, liveText, plainName, summary, more };
})();
