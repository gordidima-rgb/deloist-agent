// Значок и карточка «Бесплатный сервер» в левой панели: состояние машины Instabox.
// GET /api/cloud/instabox/status машину не будит; опрашиваем не чаще раза в минуту и только пока окно видно.

export const STATUS_URL = '/api/cloud/instabox/status';
export const POLL_MS = 60_000;
export const DISMISS_KEY = 'deloist-instabox-card-dismissed';

// Тексты состояний (ключи для tr() лежат в компонентах: check_i18n не смотрит .js).
// kind: none | running | sleeping | cooldown | offline | busy
export function ringState(status) {
  const machine = status?.machine || null;
  if (!machine || machine.state === 'missing') return { connected: false, kind: 'none' };
  if (status?.job?.state === 'running') return { connected: true, kind: 'busy' };
  switch (machine.state) {
    case 'running':
      return { connected: true, kind: 'running' };
    case 'paused':
    case 'stopped':
      return { connected: true, kind: 'sleeping' };
    case 'cooldown':
      return { connected: true, kind: 'cooldown', until: cooldownDate(machine) };
    case 'starting':
      return { connected: true, kind: 'busy' };
    default:
      return { connected: true, kind: 'offline' };
  }
}

function cooldownDate(machine) {
  const t = machine.cooldownUntil ? Date.parse(machine.cooldownUntil) : NaN;
  if (Number.isFinite(t)) return new Date(t);
  const s = Number(machine.retryAfterSeconds);
  return Number.isFinite(s) && s > 0 ? new Date(Date.now() + s * 1000) : null;
}

export function clock(date) {
  if (!date) return '';
  const locale = (typeof window !== 'undefined' && window.LOCALE) || undefined;
  return date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
}

// Карточку показываем, когда статус уже получен, машины нет и карточку не закрывали.
export function cardVisible({ loaded, status, dismissed }) {
  return !!loaded && !dismissed && !ringState(status).connected;
}

export function readDismissed() {
  try {
    return localStorage.getItem(DISMISS_KEY) === '1';
  } catch {
    return false;
  }
}

export function writeDismissed() {
  try {
    localStorage.setItem(DISMISS_KEY, '1');
  } catch {
    /* хранилище недоступно: карточка вернётся после перезапуска */
  }
}

// Общий опрос на два компонента: один запрос в минуту, пока окно видно.
const feeds = new WeakMap();

export function instaboxFeed(api, doc = typeof document !== 'undefined' ? document : null) {
  if (feeds.has(api)) return feeds.get(api);
  let state = { loaded: false, status: null };
  const subs = new Set();
  let timer = null;
  let last = 0;
  let busy = false;
  const set = (next) => {
    state = next;
    subs.forEach((fn) => fn(state));
  };
  async function refresh(force = false) {
    if (busy || (doc && doc.hidden)) return;
    if (!force && Date.now() - last < POLL_MS - 1000) return;
    busy = true;
    last = Date.now();
    try {
      set({ loaded: true, status: await api(STATUS_URL) });
    } catch {
      // Облако закрыто или нет связи с программой: ничего не показываем.
      set({ loaded: false, status: null });
    } finally {
      busy = false;
    }
  }
  const onVisible = () => refresh();
  const feed = {
    get state() {
      return state;
    },
    refresh,
    subscribe(fn) {
      subs.add(fn);
      fn(state);
      if (subs.size === 1) {
        refresh(true);
        timer = setInterval(() => refresh(), POLL_MS);
        doc?.addEventListener('visibilitychange', onVisible);
      }
      return () => {
        subs.delete(fn);
        if (!subs.size) {
          clearInterval(timer);
          doc?.removeEventListener('visibilitychange', onVisible);
        }
      };
    },
  };
  feeds.set(api, feed);
  return feed;
}
