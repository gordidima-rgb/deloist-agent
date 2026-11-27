<script module>
  // Подписи рабочих процессов в одном месте: так check_i18n видит каждый ключ.
  import { tr } from '../../lib/i18n.js';
  import { recurrenceSummary } from '../../lib/todos.js';

  export function kindTitle(kind) {
    switch (kind) {
      case 'trigger':
        return tr('Триггер');
      case 'prompt':
        return tr('Промпт');
      case 'bot':
        return tr('Бот');
      case 'llm':
        return 'LLM';
      case 'condition':
        return tr('Условие');
      case 'approval':
        return tr('Одобрение');
      case 'output':
        return tr('Вывод');
      case 'note':
        return tr('Заметка');
      case 'text':
        return tr('Текст');
      case 'asset':
        return tr('Ресурс');
      case 'image':
        return tr('Изображение');
      default:
        return kind;
    }
  }

  export function kindHint(kind) {
    switch (kind) {
      case 'trigger':
        return tr('Запускает процесс');
      case 'prompt':
        return tr('Готовый текст для следующих шагов');
      case 'bot':
        return tr('Поручение одному из ваших ботов');
      case 'llm':
        return tr('Ответ модели без бота');
      case 'condition':
        return tr('Развилка да или нет');
      case 'approval':
        return tr('Ждёт вашего решения');
      case 'output':
        return tr('Отправляет результат наружу');
      case 'asset':
        return tr('Файл, картинка или готовый текст');
      case 'image':
        return tr('Рисует картинку по описанию');
      default:
        return '';
    }
  }

  export function groupTitle(group) {
    if (group === 'inputs') return tr('Входы');
    if (group === 'steps') return tr('Шаги');
    return tr('Поток');
  }

  export function statusLabel(status) {
    switch (status) {
      case 'running':
        return tr('Идёт');
      case 'approval':
        return tr('Ждёт решения');
      case 'stopping':
        return tr('Останавливается');
      case 'done':
        return tr('Готово');
      case 'failed':
        return tr('Ошибка');
      case 'stopped':
        return tr('Остановлено');
      case 'skipped':
        return tr('Пропущено');
      default:
        return '';
    }
  }

  export function connectionText(code) {
    switch (code) {
      case 'annotation':
        return tr('Заметки и текст не соединяются с шагами');
      case 'no_output':
        return tr('У этого шага нет выхода');
      case 'no_input':
        return tr('В этот шаг нельзя вести линию');
      case 'self':
        return tr('Шаг нельзя соединить с самим собой');
      case 'duplicate':
        return tr('Эти шаги уже соединены');
      case 'type':
        return tr('Тип данных не подходит: у выхода и входа разные порты');
      case 'limit':
        return tr('Слишком много связей в процессе');
      default:
        return tr('Так соединить нельзя');
    }
  }

  export function seconds(sec) {
    if (sec < 60) return tr('{n} с', { n: sec.toFixed(sec < 10 ? 1 : 0) });
    const m = Math.floor(sec / 60);
    return tr('{m} мин {s} с', { m, s: Math.round(sec - m * 60) });
  }

  export function weekdayName(n) {
    return [tr('понедельник'), tr('вторник'), tr('среду'), tr('четверг'), tr('пятницу'), tr('субботу'), tr('воскресенье')][(n || 1) - 1];
  }

  // Дни недели рутины: «по будням», «по выходным» или список «Пн Ср».
  function daysText(days) {
    const list = [...new Set((days || []).map(Number).filter((d) => d >= 1 && d <= 7))].sort((a, b) => a - b);
    const key = list.join('');
    if (!list.length || list.length === 7) return '';
    if (key === '12345') return tr('по будням');
    if (key === '67') return tr('по выходным');
    const names = [tr('Пн'), tr('Вт'), tr('Ср'), tr('Чт'), tr('Пт'), tr('Сб'), tr('Вс')];
    return list.map((d) => names[d - 1]).join(' ');
  }

  // Расписание, как оно пришло из рутины: время с днями и правилом повтора, интервал с окном, cron, вебхук.
  function routineScheduleText(s) {
    if (s.hook) return tr('по внешнему вызову');
    if (s.kind === 'cron') return s.tz ? `${s.cron} (${s.tz})` : s.cron || tr('Не задано');
    if (s.kind === 'daily' && (s.days?.length || s.recurrence || s.start_on)) {
      const parts = [tr('в {time}', { time: s.time || '09:00' }), recurrenceSummary(s.recurrence, tr, window.LOCALE) || daysText(s.days) || tr('каждый день')];
      if (s.start_on) parts.push(tr('с {date}', { date: new Date(`${s.start_on}T00:00:00`).toLocaleDateString(window.LOCALE, { day: 'numeric', month: 'long' }) }));
      return parts.join(', ');
    }
    if (s.kind === 'interval' && s.every_minutes) {
      const low = s.every_minutes;
      const high = s.every_max_minutes || 0;
      const parts = [high > low ? tr('каждые {from}–{to} мин', { from: low, to: high }) : tr('каждые {minutes} мин', { minutes: low })];
      if (s.window_from && s.window_to) parts.push(`${s.window_from}–${s.window_to}`);
      const days = daysText(s.days);
      if (days) parts.push(days);
      return parts.join(', ');
    }
    return '';
  }

  export function scheduleText(s) {
    if (!s) return tr('Не задано');
    const fromRoutine = routineScheduleText(s);
    if (fromRoutine) return fromRoutine;
    switch (s.kind) {
      case 'hourly':
        return tr('Каждый час в :{m}', { m: String(s.minute ?? 0).padStart(2, '0') });
      case 'daily':
        return tr('Каждый день в {t}', { t: s.time || '09:00' });
      case 'weekdays':
        return tr('По будням в {t}', { t: s.time || '09:00' });
      case 'weekly':
        return tr('Каждую неделю: {d}, {t}', { d: weekdayName(s.weekday), t: s.time || '09:00' });
      case 'monthly':
        return tr('Каждый месяц, {d}-го в {t}', { d: s.day || 1, t: s.time || '09:00' });
      case 'interval': {
        const unit = s.unit === 'hours' ? tr('ч') : s.unit === 'days' ? tr('дн') : tr('мин');
        return tr('Каждые {n} {u}', { n: s.amount || 1, u: unit });
      }
      case 'once':
        return tr('Один раз: {d}, {t}', { d: s.date || '', t: s.time || '09:00' });
      default:
        return tr('Не задано');
    }
  }

  export function scheduleKindLabel(kind) {
    switch (kind) {
      case 'hourly':
        return tr('Каждый час');
      case 'daily':
        return tr('Каждый день');
      case 'weekdays':
        return tr('По будням');
      case 'weekly':
        return tr('Раз в неделю');
      case 'monthly':
        return tr('Раз в месяц');
      case 'interval':
        return tr('Через равные промежутки');
      case 'once':
        return tr('Один раз');
      default:
        return kind;
    }
  }

  // Дата следующего запуска с сервера: секунды, миллисекунды или строка.
  export function nextRunText(value) {
    if (!value) return '';
    let ms = value;
    if (typeof value === 'number') ms = value < 1e12 ? value * 1000 : value;
    else {
      const n = Number(value);
      ms = Number.isFinite(n) && String(value).trim() !== '' ? (n < 1e12 ? n * 1000 : n) : Date.parse(String(value));
    }
    if (!ms || Number.isNaN(ms)) return '';
    const d = new Date(ms);
    const day = new Date();
    const same = d.toDateString() === day.toDateString();
    const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    return same ? tr('сегодня в {t}', { t: time }) : `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })}, ${time}`;
  }

  export function agoText(value, now = Date.now()) {
    let ms = value;
    if (typeof value !== 'number') ms = Date.parse(String(value));
    else if (value < 1e12) ms = value * 1000;
    if (!ms || Number.isNaN(ms)) return '';
    const sec = Math.max(0, Math.round((now - ms) / 1000));
    if (sec < 60) return tr('только что');
    if (sec < 3600) return tr('{n} мин назад', { n: Math.round(sec / 60) });
    if (sec < 86400) return tr('{n} ч назад', { n: Math.round(sec / 3600) });
    return tr('{n} дн назад', { n: Math.round(sec / 86400) });
  }

  export function sourceLabel(source) {
    switch (source) {
      case 'schedule':
        return tr('По расписанию');
      case 'webhook':
        return tr('Вебхук');
      case 'telegram':
        return 'Telegram';
      case 'discord':
        return 'Discord';
      case 'slack':
        return 'Slack';
      case 'node':
        return tr('Один шаг');
      default:
        return tr('Вручную');
    }
  }

  export function triggerLabel(id) {
    switch (id) {
      case 'schedule':
        return tr('Расписание');
      case 'webhook':
        return tr('Вебхук');
      case 'telegram':
        return 'Telegram';
      case 'discord':
        return 'Discord';
      case 'slack':
        return 'Slack';
      default:
        return tr('Вручную');
    }
  }

  export function targetLabel(id) {
    switch (id) {
      case 'chat':
        return tr('Чат бота');
      case 'telegram':
        return 'Telegram';
      case 'discord':
        return 'Discord';
      case 'slack':
        return 'Slack';
      case 'file':
        return tr('Файл');
      default:
        return tr('Уведомление');
    }
  }

  // Значок источника запуска в истории и списке.
  export function sourceIcon(source) {
    switch (source) {
      case 'schedule':
        return 'calendar';
      case 'webhook':
        return 'webhook';
      case 'telegram':
      case 'discord':
      case 'slack':
        return source;
      case 'node':
        return 'run_step';
      default:
        return 'manual';
    }
  }

  export function versionKindLabel(kind) {
    switch (kind) {
      case 'run':
        return tr('Перед запуском');
      case 'restore':
        return tr('До восстановления');
      case 'manual':
        return tr('Сохранено вручную');
      default:
        return tr('Автосохранение');
    }
  }

  // Время события: «сегодня в 12:04», «вчера в 09:30» или дата.
  export function whenText(ms) {
    if (!ms) return '';
    const d = new Date(ms);
    const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    const day = new Date();
    if (d.toDateString() === day.toDateString()) return tr('сегодня в {t}', { t: time });
    day.setDate(day.getDate() - 1);
    if (d.toDateString() === day.toDateString()) return tr('вчера в {t}', { t: time });
    return `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })}, ${time}`;
  }
</script>
