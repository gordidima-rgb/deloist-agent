// Каналы напоминаний для вкладок «Напоминания» и «Привычки» (как CHANNELS в src/reminders.rs).
// Флаги Windows и Telegram — то, что видит человек; список — то, что уходит в API.

// Флаги и доступность Telegram -> список каналов. Telegram попадает в список, только если чат привязан.
export function channelsOf({ windows, telegram, paired }) {
  const list = [];
  if (windows) list.push('windows');
  if (telegram && paired) list.push('telegram');
  return list;
}

// Список каналы с сервера -> флаги. Пустой список означает Windows (так решает сервер).
export function flagsOf(channels) {
  const list = Array.isArray(channels) && channels.length ? channels : ['windows'];
  return { windows: list.includes('windows'), telegram: list.includes('telegram') };
}

// Telegram доступен, когда чат привязан. Статус спрашиваем у /api/telegram; сбой считаем «не привязан».
export async function isTelegramPaired(api) {
  try {
    const status = await api('/api/telegram');
    return Boolean(status?.chatPaired);
  } catch {
    return false;
  }
}
