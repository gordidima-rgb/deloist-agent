// Помощники экранов «Устройства» и «другие ПК». Всё, что пришло с другого ПК или устройства, — чужие данные:
// в адреса попадает только через enc(), в стили — только через safeColor(), в разметку — только текстом.
import { tr } from './i18n.js';
import { whenText } from './when.js';

// Любой чужой id в адресе страницы.
export const enc = (value) => encodeURIComponent(String(value ?? ''));

// Цвет бота с чужого ПК: только #rgb … #rrggbbaa, иначе цвет кита.
export const safeColor = (value) => (/^#[0-9a-fA-F]{3,8}$/.test(String(value ?? '')) ? String(value) : '');

// 6 цифр → «123 456».
export const groupCode = (code) => {
  const digits = String(code ?? '').replace(/\D/g, '');
  return digits.length === 6 ? `${digits.slice(0, 3)} ${digits.slice(3)}` : digits;
};

export const digitsOnly = (text) => String(text ?? '').replace(/\D/g, '').slice(0, 6);

export const kindLabel = (kind) => (kind === 'pc' ? tr('ПК') : tr('Телефон или браузер'));

// «только что», «5 мин назад», «3 ч назад», иначе дата.
export function agoText(ms, now = Date.now()) {
  const at = Number(ms);
  if (!Number.isFinite(at) || at <= 0) return tr('ещё не выходило на связь');
  const minutes = Math.floor((now - at) / 60000);
  if (minutes < 1) return tr('только что');
  if (minutes < 60) return tr('{n} мин назад', { n: minutes });
  if (minutes < 60 * 24) return tr('{n} ч назад', { n: Math.floor(minutes / 60) });
  return whenText(at);
}

// Сообщение ошибки как текст (с другого ПК оно уже очищено; в разметку не идёт).
export const errorText = (err) => String(err?.message || tr('Не удалось выполнить запрос'));

// Состояние связи этого ПК с deloist.ru.
export function linkText(info) {
  if (!info) return '';
  if (info.connected) return tr('Подключено к deloist.ru');
  switch (info.status) {
    case 'elsewhere':
      return tr('Этот аккаунт подключён на другом ПК: здесь связь выключена');
    case 'device_limit':
      return tr('Достигнут предел устройств аккаунта: освободите место на сайте');
    case 'bad_proof':
      return tr('Сервер не принял подпись этого ПК');
    case 'version':
      return tr('Эта версия программы устарела для связи с сайтом');
    case 'rejected':
      return tr('Сервер отклонил подключение');
    default:
      return tr('Нет связи с deloist.ru, пробуем снова');
  }
}
