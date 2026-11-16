// Вложения в чате: скрепка, перетаскивание, вставка картинки, ленты файлов в поле ввода и в переписке.
// Классический скрипт: app.js берёт готовое из window.Attach и ничего не знает о деталях.
(() => {
  const MAX_BYTES = 10 * 1024 * 1024;
  const MAX_COUNT = 10;
  const IMAGE_EXT = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif'];
  const T = (text, vars) => (window.tr ? window.tr(text, vars) : text);

  const ext = (name) => String(name || '').split('.').pop().toLowerCase();
  const isImage = (file) => IMAGE_EXT.includes(ext(file.name)) || String(file.type || '').startsWith('image/');

  function sizeLabel(bytes) {
    const n = Number(bytes) || 0;
    if (n < 1024) return T('{n} Б', { n });
    if (n < 1024 * 1024) return T('{n} КБ', { n: Math.round(n / 1024) });
    return T('{n} МБ', { n: (n / (1024 * 1024)).toFixed(1) });
  }

  const CLIP =
    DeloistUI.icon('paperclip', { className: 'i' });
  const CLOSE =
    DeloistUI.icon('close', { className: 'i' });

  // Плашка формата: расширение файла на подложке цвета его рода (таблица, документ, код…).
  const KINDS = {
    table: ['csv', 'tsv', 'xls', 'xlsx', 'xlsm', 'ods', 'numbers', 'dbf', 'mdb', 'accdb', 'sqlite', 'db'],
    book: ['epub', 'fb2', 'mobi', 'azw', 'azw3', 'djvu', 'cbz'],
    app: ['exe', 'msi', 'msix', 'apk', 'aab', 'ipa', 'dmg', 'pkg', 'deb', 'rpm', 'appimage', 'bat', 'cmd'],
    pdf: ['pdf'],
    doc: ['doc', 'docx', 'odt', 'rtf', 'pages', 'ppt', 'pptx', 'key', 'odp'],
    text: ['txt', 'md', 'log', 'ini', 'cfg', 'conf', 'env'],
    code: ['js', 'mjs', 'ts', 'tsx', 'jsx', 'py', 'rs', 'go', 'java', 'kt', 'c', 'h', 'cpp', 'cs', 'rb', 'php', 'sh', 'ps1', 'sql', 'html', 'css', 'svelte', 'vue', 'json', 'yaml', 'yml', 'toml', 'xml'],
    archive: ['zip', 'rar', '7z', 'tar', 'gz', 'tgz', 'bz2', 'xz'],
    media: ['mp3', 'wav', 'ogg', 'oga', 'opus', 'flac', 'ape', 'aac', 'm4a', 'wma', 'aiff', 'mid', 'mp4', 'm4v', 'mov', 'webm', 'mkv', 'avi', 'wmv', 'flv', 'mpg', 'mpeg', '3gp'],
    image: IMAGE_EXT.concat(['svg', 'ico', 'heic', 'heif', 'tif', 'tiff', 'jxl']),
    design: ['psd', 'psb', 'ai', 'eps', 'indd', 'idml', 'xd', 'aep', 'prproj', 'cdr', 'cmx', 'cdt', 'fig', 'sketch', 'afdesign', 'afphoto', 'afpub', 'xcf', 'kra'],
    raw: ['raw', 'cr2', 'cr3', 'nef', 'arw', 'dng', 'orf', 'rw2', 'raf'],
  };
  // Свои цвета у форматов, которые узнают по цвету: Adobe и Corel.
  const BRAND = { psd: 'ps', psb: 'ps', ai: 'ai', eps: 'ai', indd: 'id', idml: 'id', xd: 'xd', aep: 'ae', prproj: 'pr', cdr: 'cdr', cmx: 'cdr', cdt: 'cdr' };
  const SHORT = { prproj: 'PR', afdesign: 'AFD', afphoto: 'AFP', afpub: 'AFPB', sketch: 'SKCH' };
  function badge(fileName) {
    const e = ext(fileName);
    const known = String(fileName || '').includes('.') && e.length <= 8;
    const kind = (known && Object.keys(KINDS).find((k) => KINDS[k].includes(e))) || 'other';
    const icon = document.createElement('span');
    icon.className = 'attach-icon attach-type';
    icon.dataset.kind = kind;
    if (BRAND[e]) icon.dataset.brand = BRAND[e];
    if (known) icon.textContent = SHORT[e] || e.slice(0, 4);
    else icon.innerHTML = CLIP;
    icon.setAttribute('aria-hidden', 'true');
    return icon;
  }

  // У снимка из буфера обмена имя пустое или «image.png»: даём понятное, с временем.
  function nameOf(file) {
    const name = (file.name || '').trim();
    if (name && name !== 'image.png') return name;
    const now = new Date();
    const pad = (n) => String(n).padStart(2, '0');
    const stamp = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
    return `${T('снимок')}-${stamp}.${ext(name) || (file.type || '').split('/')[1] || 'png'}`;
  }

  const readBase64 = (file) =>
    new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(',')[1] || '');
      reader.onerror = () => reject(new Error(T('Не удалось прочитать файл')));
      reader.readAsDataURL(file);
    });

  const rawUrl = (ownerId, path) => `/api/bots/${encodeURIComponent(ownerId)}/file/raw?path=${encodeURIComponent(path)}`;

  // Большая картинка по щелчку: простое наложение, закрывается щелчком или Esc.
  function lightbox(src, name) {
    const wrap = document.createElement('div');
    wrap.className = 'attach-lightbox';
    wrap.tabIndex = -1;
    const img = document.createElement('img');
    img.src = src;
    img.alt = name;
    wrap.append(img);
    const close = () => {
      wrap.remove();
      removeEventListener('keydown', onKey, true);
    };
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      close();
    };
    wrap.addEventListener('click', close);
    addEventListener('keydown', onKey, true);
    document.body.append(wrap);
    wrap.focus();
  }

  // Вложения отправленного сообщения: миниатюра картинки или карточка файла со ссылкой.
  function sent(files, ownerId) {
    const box = document.createElement('div');
    box.className = 'attach-sent';
    if (!ownerId) return box;
    for (const file of files) {
      const url = rawUrl(ownerId, file.path);
      if (file.kind === 'image') {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'attach-pic';
        button.title = file.name;
        button.setAttribute('aria-label', `${T('Открыть')}: ${file.name}`);
        const img = document.createElement('img');
        img.loading = 'lazy';
        img.alt = file.name;
        img.src = url;
        button.append(img);
        button.addEventListener('click', () => lightbox(url, file.name));
        box.append(button);
        continue;
      }
      const link = document.createElement('a');
      link.className = 'attach-chip sent';
      link.href = url;
      link.download = file.name;
      link.title = `${file.name} — ${T('скачать')}`;
      const icon = badge(file.name);
      const name = document.createElement('span');
      name.className = 'attach-name';
      name.textContent = file.name;
      const size = document.createElement('span');
      size.className = 'attach-size';
      size.textContent = sizeLabel(file.size);
      link.append(icon, name, size);
      box.append(link);
    }
    return box;
  }

  // ---- Результат работы бота в переписке: картинка, звук, видео с кнопками ----

  const AUDIO_EXT = ['mp3', 'wav', 'ogg', 'oga', 'opus', 'm4a', 'aac', 'flac'];
  const VIDEO_EXT = ['mp4', 'm4v', 'webm', 'mov'];
  const mediaKind = (name) => {
    const e = ext(name);
    return IMAGE_EXT.includes(e) ? 'image' : AUDIO_EXT.includes(e) ? 'audio' : VIDEO_EXT.includes(e) ? 'video' : '';
  };

  // Ссылка на файл из воркспейса бота: file:///…/workspaces/<бот>/<путь> или путь относительно папки бота.
  function resolveRef(target, ownerId) {
    let raw = String(target || '').trim();
    if (!raw || /^(https?|data|javascript):/i.test(raw)) return null;
    try {
      raw = decodeURIComponent(raw.replace(/^file:\/\/\/?/i, ''));
    } catch {
      return null;
    }
    raw = raw.replace(/\\/g, '/');
    const m = raw.match(/(?:^|\/)workspaces\/([^/]+)\/(.+)$/);
    let owner = ownerId;
    let rel = raw;
    if (m) {
      owner = m[1];
      rel = m[2];
    } else if (/^[a-z]:\//i.test(raw) || raw.startsWith('/')) {
      return null;
    }
    rel = rel.replace(/^\.\//, '');
    if (!owner || !rel || rel.split('/').includes('..') || !mediaKind(rel)) return null;
    return { owner, path: rel, name: rel.split('/').pop(), kind: mediaKind(rel) };
  }

  // Достаёт из текста ответа ссылки на медиафайлы: в тексте остаётся подпись, файлы уходят в карточки.
  function extractResults(text, ownerId) {
    const files = [];
    const seen = new Set();
    const take = (target) => {
      const ref = resolveRef(target, ownerId);
      if (!ref) return false;
      const key = `${ref.owner}/${ref.path}`;
      if (!seen.has(key)) {
        seen.add(key);
        files.push(ref);
      }
      return true;
    };
    let out = String(text ?? '').replace(/!?\[([^\]\n]*)\]\(\s*([^)\s]+)\s*\)/g, (whole, label, target) =>
      take(target) ? label : whole,
    );
    out = out.replace(/file:\/\/\/[^\s)<>`]+/gi, (url) => (take(url) ? '' : url));
    return { text: out, files };
  }

  const fileUrl = (ref, download) => `${rawUrl(ref.owner, ref.path)}${download ? '&download=1' : ''}`;

  const COPY_ICON = DeloistUI.icon('copy-wide', { className: 'i' });
  const DOWNLOAD_ICON = DeloistUI.icon('download', { className: 'i' });
  const SEND_ICON = DeloistUI.icon('send-plane', { className: 'i' });

  // Картинка в буфер: браузер принимает только PNG, остальное перерисовываем.
  async function copyImage(url) {
    const blob = await (await fetch(url)).blob();
    let png = blob;
    if (blob.type !== 'image/png') {
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width;
      canvas.height = bitmap.height;
      canvas.getContext('2d').drawImage(bitmap, 0, 0);
      png = await new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('png'))), 'image/png'));
    }
    await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
  }

  // `ctx`: { bots(ownerId) → [{id, name}], send(botId, ref) → промис, notify(text) }.
  function resultCards(files, ctx = {}) {
    const box = document.createElement('div');
    box.className = 'result-cards';
    const notify = ctx.notify || (() => {});
    for (const ref of files) {
      const card = document.createElement('div');
      card.className = `result-card ${ref.kind}`;
      const url = fileUrl(ref);
      let media;
      if (ref.kind === 'image') {
        media = document.createElement('button');
        media.type = 'button';
        media.className = 'result-pic';
        media.setAttribute('aria-label', `${T('Открыть')}: ${ref.name}`);
        const img = document.createElement('img');
        img.loading = 'lazy';
        img.alt = ref.name;
        img.src = url;
        media.append(img);
        media.addEventListener('click', () => lightbox(url, ref.name));
      } else {
        media = document.createElement(ref.kind === 'video' ? 'video' : 'audio');
        media.className = 'result-player';
        media.controls = true;
        media.preload = 'metadata';
        media.src = url;
      }

      const bar = document.createElement('div');
      bar.className = 'result-bar';
      const label = document.createElement('span');
      label.className = 'result-name';
      label.textContent = ref.name;
      label.title = ref.path;

      const button = (icon, text, onClick) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'ghost result-btn';
        b.innerHTML = icon;
        const span = document.createElement('span');
        span.textContent = text;
        b.append(span);
        b.addEventListener('click', onClick);
        return b;
      };

      const copy = button(COPY_ICON, T('Скопировать'), async () => {
        try {
          if (ref.kind === 'image') {
            await copyImage(url);
            notify(T('Картинка скопирована'));
          } else {
            await navigator.clipboard.writeText(ref.path);
            notify(T('Путь к файлу скопирован'));
          }
        } catch {
          notify(T('Не удалось скопировать'));
        }
      });

      const save = document.createElement('a');
      save.className = 'ghost result-btn';
      save.href = fileUrl(ref, true);
      save.download = ref.name;
      save.innerHTML = DOWNLOAD_ICON;
      const saveText = document.createElement('span');
      saveText.textContent = T('Скачать');
      save.append(saveText);

      const send = button(SEND_ICON, T('Другому боту'), () => {
        const open = card.querySelector('.result-menu');
        if (open) {
          open.remove();
          return;
        }
        const menu = document.createElement('div');
        menu.className = 'result-menu';
        const bots = ctx.bots ? ctx.bots(ref.owner) : [];
        if (!bots.length) {
          const empty = document.createElement('div');
          empty.className = 'result-menu-empty';
          empty.textContent = T('Других ботов пока нет');
          menu.append(empty);
        }
        for (const bot of bots) {
          const item = document.createElement('button');
          item.type = 'button';
          item.className = 'ghost';
          item.textContent = bot.name;
          item.addEventListener('click', async () => {
            menu.remove();
            try {
              await ctx.send(bot.id, ref);
              notify(T('Отправлено боту {name}', { name: bot.name }));
            } catch (err) {
              notify(err?.message || T('Не удалось отправить'));
            }
          });
          menu.append(item);
        }
        card.append(menu);
      });

      bar.append(label, copy, save, send);
      card.append(media, bar);
      box.append(card);
    }
    return box;
  }

  // Поле ввода с вложениями. `endpoint()` отдаёт адрес загрузки для открытого чата или null, если прикладывать некуда.
  function composer({ root, textarea, pick, picker, list, hint, endpoint, post, notify, onChange }) {
    let items = [];
    let depth = 0;

    const usable = () => Boolean(endpoint());

    function render() {
      list.replaceChildren();
      list.hidden = !items.length;
      items.forEach((item, index) => {
        const chip = document.createElement('div');
        chip.className = 'attach-chip';
        if (item.image && item.saved) {
          // Файл уже лежит у бота (снимок экрана): превью — по его адресу.
          const thumb = document.createElement('img');
          thumb.className = 'attach-thumb';
          thumb.alt = '';
          thumb.src = item.url;
          chip.append(thumb);
        } else if (item.image) {
          const thumb = document.createElement('img');
          thumb.className = 'attach-thumb';
          thumb.alt = '';
          // Политика страницы пускает картинки только со своего адреса и data:, blob: она режет.
          // Превью читаем один раз и держим на элементе: перерисовка ленты не читает файл заново.
          // Превью с меткой формата в углу; что браузер показать не может (PSD, TIFF…), остаётся плашкой.
          const box = document.createElement('span');
          box.className = 'attach-thumb-box';
          const label = document.createElement('span');
          label.className = 'attach-ext';
          label.textContent = ext(item.name).slice(0, 4);
          box.append(thumb, label);
          if (item.preview) thumb.src = item.preview;
          else {
            const reader = new FileReader();
            reader.onload = () => {
              item.preview = String(reader.result);
              thumb.src = item.preview;
            };
            reader.onerror = () => box.replaceWith(badge(item.name));
            reader.readAsDataURL(item.file);
          }
          thumb.addEventListener('error', () => box.replaceWith(badge(item.name)), { once: true });
          chip.append(box);
        } else {
          chip.append(badge(item.name));
        }
        const name = document.createElement('span');
        name.className = 'attach-name';
        name.textContent = item.name;
        name.title = item.name;
        const size = document.createElement('span');
        size.className = 'attach-size';
        size.textContent = sizeLabel(item.saved ? item.size || 0 : item.file.size);
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'attach-x';
        remove.title = T('Убрать');
        remove.setAttribute('aria-label', `${T('Убрать')}: ${item.name}`);
        remove.innerHTML = CLOSE;
        remove.addEventListener('click', () => {
          items.splice(index, 1);
          render();
          onChange();
          textarea.focus();
        });
        chip.append(name, size, remove);
        list.append(chip);
      });
    }

    function add(files) {
      if (!usable()) return;
      let extra = 0;
      for (const file of [...(files || [])]) {
        if (items.length >= MAX_COUNT) {
          extra += 1;
        } else if (!file.size) {
          notify(T('Пустой файл не приложить: {name}', { name: file.name || '' }));
        } else if (file.size > MAX_BYTES) {
          notify(T('Файл больше 10 МБ: {name}', { name: file.name || '' }));
        } else {
          items.push({ file, name: nameOf(file), image: isImage(file) });
        }
      }
      if (extra) notify(T('К одному сообщению не больше 10 файлов'));
      render();
      onChange();
    }

    pick.addEventListener('click', () => picker.click());
    picker.addEventListener('change', () => {
      add(picker.files);
      picker.value = '';
    });

    // Перетаскивание над всем чатом; счётчик глубины, потому что dragleave приходит и от вложенных элементов.
    const hasFiles = (e) => e.dataTransfer && [...e.dataTransfer.types].includes('Files');
    root.addEventListener('dragenter', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth += 1;
      root.classList.toggle('dropping', usable());
    });
    root.addEventListener('dragover', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = usable() ? 'copy' : 'none';
    });
    root.addEventListener('dragleave', (e) => {
      if (!hasFiles(e)) return;
      depth = Math.max(0, depth - 1);
      if (!depth) root.classList.remove('dropping');
    });
    root.addEventListener('drop', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth = 0;
      root.classList.remove('dropping');
      add(e.dataTransfer.files);
    });
    // Картинка из буфера обмена. Обычный текст вставляется как всегда.
    textarea.addEventListener('paste', (e) => {
      const pasted = [...((e.clipboardData && e.clipboardData.files) || [])];
      if (!pasted.length || !usable()) return;
      e.preventDefault();
      add(pasted);
    });
    hint.textContent = T('Отпусти файл, чтобы приложить к сообщению');

    return {
      count: () => items.length,
      has: () => items.length > 0,
      snapshot: () => items.slice(),
      restore(saved) {
        if (!items.length) {
          items = saved.slice();
          render();
          onChange();
        }
      },
      clear() {
        items = [];
        render();
        onChange();
      },
      // Уже сохранённый у бота файл (снимок экрана с горячей клавиши): загружать его заново не нужно.
      addSaved({ name, path, kind, size, url }) {
        if (!path || items.length >= MAX_COUNT) return;
        items.push({ saved: true, name: name || path.split('/').pop(), path, size, url, image: kind === 'image' });
        render();
        onChange();
      },
      // Файлы уезжают в папку бота по одному; в сообщение идут только пути.
      // Адрес загрузки берёт вызывающий в момент отправки: пока файлы едут, человек мог открыть другой чат.
      async upload(saved, url) {
        const paths = [];
        for (const item of saved) {
          if (item.saved) {
            paths.push(item.path);
            continue;
          }
          const data = await readBase64(item.file);
          const res = await post(url, { name: item.name, data });
          paths.push(res.path);
        }
        return paths;
      },
    };
  }

  window.Attach = { composer, sent, sizeLabel, extractResults, resultCards };
})();
