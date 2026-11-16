// Уведомления кита: один хост на документ и toast(message, { action }).
// Очередь обычная (не руны), чтобы файл оставался .js и им могли пользоваться старые страницы.
import { mount } from 'svelte';
import ToastHost from '../components/kit/ToastHost.svelte';

const items = [];
const listeners = new Set();
let seq = 0;
let host = null;

function snapshot() {
  return items.map((item) => ({ ...item }));
}

function emit() {
  const next = snapshot();
  for (const listener of listeners) listener(next);
}

export function subscribeToasts(listener) {
  listeners.add(listener);
  listener(snapshot());
  return () => listeners.delete(listener);
}

function ensureHost() {
  if (host || typeof document === 'undefined' || !document.body) return;
  const target = document.createElement('div');
  target.className = 'k-toast-portal';
  document.body.append(target);
  host = {
    target,
    instance: mount(ToastHost, {
      target,
      props: { subscribe: subscribeToasts, dismiss: dismissToast },
    }),
  };
}

export function toast(message, { action = null, duration = 5000 } = {}) {
  ensureHost();
  const id = ++seq;
  items.push({ id, message, action, duration });
  emit();
  if (duration > 0) setTimeout(() => dismissToast(id), duration);
  return id;
}

export function dismissToast(id) {
  const at = items.findIndex((item) => item.id === id);
  if (at < 0) return;
  items.splice(at, 1);
  emit();
}
