// Файлы процесса: загрузка ресурса на сервер и адрес файла для показа.
export const ASSET_LIMIT = 20 * 1024 * 1024;

export function fileUrlFor(workflowId, path) {
  const p = String(path || '').trim();
  if (/^(data:|blob:|https?:|\/)/.test(p)) return p;
  const rel = p
    .split('/')
    .filter(Boolean)
    .map(encodeURIComponent)
    .join('/');
  return `/api/workflows/${encodeURIComponent(workflowId)}/files/${rel}`;
}

export function toDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error || new Error('read'));
    reader.onload = () => resolve(String(reader.result || ''));
    reader.readAsDataURL(file);
  });
}

export async function toBase64(file) {
  const text = await toDataUrl(file);
  return text.slice(text.indexOf(',') + 1);
}

/** POST /api/workflows/{id}/assets {name, data, mime} → {name, path, mime, size}. */
export async function uploadAssetTo(api, workflowId, file) {
  if (file.size > ASSET_LIMIT) {
    const err = new Error('файл больше 20 МБ');
    err.status = 413;
    throw err;
  }
  const data = await toBase64(file);
  return api(`/api/workflows/${encodeURIComponent(workflowId)}/assets`, {
    method: 'POST',
    body: { name: file.name, data, mime: file.type || undefined },
  });
}

/** Куда ляжет файл вывода: части пути внутри папки файлов процесса (пусто, пока путь не задан). */
export function outputParts(path) {
  return String(path || '')
    .trim()
    .replace(/[\\]/g, '/')
    .split('/')
    .filter(Boolean);
}

/** Сервер отвергает такие пути: абсолютные, с диском (:) и с «..». */
export function badOutputPath(path) {
  const p = String(path || '').trim();
  if (!p) return false;
  return p.startsWith('/') || p.startsWith('\\') || p.includes(':') || outputParts(p).some((x) => x === '..' || x === '.');
}
