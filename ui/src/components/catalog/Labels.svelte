<script module>
  // Shared labels for the catalog. They live in a .svelte file so check_i18n sees every key.
  import { tr } from '../../lib/i18n.js';

  export function categoryLabel(category) {
    switch (category) {
      case 'productivity':
        return tr('Продуктивность');
      case 'docs':
        return tr('Документы');
      case 'research':
        return tr('Исследования');
      case 'business':
        return tr('Бизнес');
      case 'marketing':
        return tr('Маркетинг');
      case 'dev':
        return tr('Разработка');
      case 'personal':
        return tr('Личное');
      case 'design':
        return tr('Дизайн');
      default:
        return category;
    }
  }

  export function pageTitle(page) {
    if (page === 'agents') return tr('Агенты');
    if (page === 'apps') return tr('Приложения');
    return tr('Навыки');
  }

  export function pageLead(page) {
    if (page === 'agents') return tr('Готовые боты с ролью, характером и навыками. Модель остаётся вашей.');
    if (page === 'apps') return tr('Сервисы, к которым бот подключается сам или работает через ваш кабинет в браузере.');
    return tr('Способы работы, которые бот получает целиком: подключите их нужным ботам.');
  }

  export function tierLabel(tier) {
    return tier === 'verified' ? tr('Проверено') : tr('Сообщество');
  }

  /** Card button text by item state. */
  export function actionLabel(item) {
    const state = item.state || {};
    if (state.update) return tr('Обновить');
    if (item.kind === 'app') {
      if (state.installed) return tr('Подключено');
      if (state.status === 'pending') return tr('Продолжить');
      return tr('Подключить');
    }
    return state.installed ? tr('Добавлено') : tr('Добавить');
  }

  export function builtinLabel(name) {
    if (name === 'browser') return tr('Браузер бота');
    if (name === 'files') return tr('Файлы бота');
    return name;
  }

  export function hintLabel(hint) {
    if (hint === 'strong' || hint === 'smart') return tr('Лучше с умной моделью');
    if (hint === 'fast') return tr('Хватит быстрой модели');
    return '';
  }
</script>
