// Ссылка на файл из папки бота для текста сообщения: путь в обратных кавычках, у папки — косая в конце.
// Тот же тип данных ловит чат, когда файл бросают не в поле ввода, а в ленту сообщений.
export const FILE_MIME = 'application/x-deloist-file';

export function fileRef(item) {
  const path = String(item?.path || item?.name || '').replace(/^\/+/, '');
  return `\`${item?.dir ? `${path.replace(/\/+$/, '')}/` : path}\``;
}
