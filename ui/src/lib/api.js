// Fetch wrapper for the app's own API: same origin, the window session cookie goes along.
// A non-2xx answer throws an Error with .status and .data (the server's JSON body).
export async function api(path, { method = 'GET', body } = {}) {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  let data = {};
  try {
    data = await res.json();
  } catch {}
  if (!res.ok) {
    const err = new Error(data.error || res.statusText || String(res.status));
    err.status = res.status;
    err.data = data;
    throw err;
  }
  return data;
}

// Server messages are Russian; web/i18n.js knows how to show them in English.
export function serverText(message) {
  const text = String(message ?? '');
  return typeof window !== 'undefined' && typeof window.trServer === 'function' ? window.trServer(text) : text;
}

export function lang() {
  return typeof window !== 'undefined' && window.LANG === 'en' ? 'en' : 'ru';
}

// Manifest text: a string or {ru, en}. Falls back to Russian.
export function loc(value) {
  if (!value) return '';
  if (typeof value === 'string') return value;
  return value[lang()] ?? value.ru ?? Object.values(value)[0] ?? '';
}

export function iconOf(item) {
  const icon = String(item?.icon || '');
  return icon.startsWith('emoji:') ? icon.slice(6) : '';
}

export const CATEGORIES = ['productivity', 'docs', 'research', 'business', 'marketing', 'dev', 'personal', 'design'];
