// Все значки интерфейса живут здесь, и больше нигде. Остальные файлы берут значок по имени:
// в Svelte — <Icon name="search" />, в обычных страницах — DeloistUI.icon('search', { size, stroke, className }).
// Правила кита (docs/design/kit.md, «Иконки»): линейный SVG в стиле Lucide, 24x24, без заливки,
// цвет берётся из текста рядом (currentColor), толщина 2 — в меню навигации, у поиска и справки.
// В ICONS лежит только содержимое svg (path, circle, rect): ни fill, ни stroke, ни цветов.
// Не сюда: кольца лимитов, ручка дока, логотипы брендов и рисунки холста процессов — они остаются на месте.

const FILE = '<path d="M6 3h8l4 4v14H6zM14 3v4h4"/>';

export const ICONS = {
  // Стрелки и шевроны
  'arrow-down': '<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>',
  'arrow-right': '<path d="M5 12h14M13 6l6 6-6 6"/>',
  'arrow-up': '<path d="M12 19V5"/><path d="m5 12 7-7 7 7"/>',
  'arrows-swap': '<path d="M7 7h10"/><path d="m14 4 3 3-3 3"/><path d="M17 17H7"/><path d="m10 14-3 3 3 3"/>',
  'chevron-down': '<path d="m6 9 6 6 6-6"/>',
  'chevron-left': '<path d="m15 6-6 6 6 6"/>',
  'chevron-right': '<path d="m9 6 6 6-6 6"/>',
  'chevron-up': '<path d="m7 14 5-5 5 5"/>',
  'chevrons-left': '<path d="m11 17-5-5 5-5M18 17l-5-5 5-5"/>',
  'chevrons-right': '<path d="m13 17 5-5-5-5M6 17l5-5-5-5"/>',
  expand: '<path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>',
  shrink: '<path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7"/>',

  // Действия
  'alert-circle': '<path d="M12 8v5"/><path d="M12 17h.01"/><circle cx="12" cy="12" r="9"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  'check-long': '<path d="m5 12 5 5 9-10"/>',
  'check-soft': '<path d="m5.5 12.5 4.2 4.2L18.5 7.6"/>',
  close: '<path d="M18 6 6 18M6 6l12 12"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h9"/>',
  'copy-wide': '<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>',
  download: '<path d="M12 3v12m0 0-4-4m4 4 4-4M4 21h16"/>',
  edit: '<path d="M4 20h4l10-10-4-4L4 16z"/><path d="m13.5 6.5 4 4"/>',
  exclamation: '<path d="M12 5v8"/><path d="M12 18.6v.1"/>',
  minimize: '<path d="M5 19h14"/>',
  more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
  pencil: '<path d="M4 20l1-4L16.5 4.5a2.1 2.1 0 0 1 3 3L8 19z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>',
  'search-short': '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/>',
  send: '<path d="M12 19V5"/><path d="m6 11 6-6 6 6"/>',
  'send-plane': '<path d="m22 2-7 20-4-9-9-4zM22 2 11 13"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  'stop-large': '<rect x="5" y="5" width="14" height="14" rx="2"/>',
  'stop-small': '<rect x="7" y="7" width="10" height="10" rx="1.5"/>',
  tick: '<path d="M20 6 9 17l-5-5"/>',
  trash: '<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="m6 7 1 13h10l1-13"/>',
  'trash-can': '<path d="M4 7h16M10 11v6M14 11v6"/><path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>',

  // Разделы, вещи и понятия
  bot: '<path d="M12 8V4H8"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2M20 14h2M15 13v2M9 13v2"/>',
  cpu: '<rect x="5" y="5" width="14" height="14" rx="2"/><path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3"/>',
  database: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/>',
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
  puzzle: '<path d="M19.4 14.9c-.2.4.1.9.6 1a2.5 2.5 0 1 0 0-4.9c-.5 0-.8.6-.6 1M19 12V8a1 1 0 0 0-1-1h-4c.4-.2.7-.6.7-1a2.5 2.5 0 1 0-5 0c0 .4.3.8.7 1H6a1 1 0 0 0-1 1v4c.2-.4.6-.7 1-.7a2.5 2.5 0 1 1 0 5c-.4 0-.8-.3-1-.7V20a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1z"/>',
  target: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/>',
  zap: '<path d="M13 2 3 14h9l-1 8 10-12h-9z"/>',
  'apps-add': '<rect x="3.5" y="3.5" width="7" height="7" rx="2"/><rect x="13.5" y="3.5" width="7" height="7" rx="2"/><rect x="3.5" y="13.5" width="7" height="7" rx="2"/><path d="M17 13.5v7M13.5 17h7"/>',
  bell: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  'bell-off': '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/><path d="m3 3 18 18"/>',
  'bell-small': '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
  box: '<path d="M4 7.5 12 3l8 4.5v9L12 21l-8-4.5z"/><path d="M4 7.5 12 12l8-4.5M12 12v9"/>',
  brain: '<path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 1V4a3 3 0 0 0-3 0z"/><path d="M15 4a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 1"/>',
  'brain-halves': '<path d="M12 5a3 3 0 0 0-5.8-1A3 3 0 0 0 4 9a3 3 0 0 0 0 6 3 3 0 0 0 2.2 5A3 3 0 0 0 12 19z"/><path d="M12 5a3 3 0 0 1 5.8-1A3 3 0 0 1 20 9a3 3 0 0 1 0 6 3 3 0 0 1-2.2 5A3 3 0 0 1 12 19z"/><path d="M12 5v14"/>',
  branch: '<circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 8v8"/><path d="M18 10c0 4-4 6-8 6"/>',
  calendar: '<rect x="4" y="5" width="16" height="15" rx="3"/><path d="M8 3v4M16 3v4M4 10h16"/>',
  'chart-bars': '<path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/>',
  'chart-columns': '<path d="M5 20V11M12 20V4M19 20v-6"/>',
  'chat-bubble': '<path d="M21 12a8 8 0 0 1-8 8H4l2.2-3.3A8 8 0 1 1 21 12z"/>',
  'chat-wave': '<path d="M12 3.5a8.5 8.5 0 1 1-4.3 15.8L3.5 20.5l1.2-4A8.5 8.5 0 0 1 12 3.5z"/><path d="M9 10.2v3.6M12 8.2v7.6M15 10.2v3.6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  compact: '<path d="M4 8h16M4 16h16M9 12h6"/>',
  'compact-lines': '<path d="M4 6h16M7 12h10M10 18h4"/>',
  'dock-hide': '<path d="M15 4h4v16h-4"/><path d="M4 12h10"/><path d="m10 8 4 4-4 4"/>',
  eye: '<path d="M2.5 12S6 5 12 5s9.5 7 9.5 7-3.5 7-9.5 7-9.5-7-9.5-7Z"/><circle cx="12" cy="12" r="2.8"/>',
  'eye-off': '<path d="M3 3l18 18"/><path d="M10.6 5.1A10 10 0 0 1 12 5c6 0 9.5 7 9.5 7a17 17 0 0 1-3 3.9M6.3 6.3A17 17 0 0 0 2.5 12S6 19 12 19a9.6 9.6 0 0 0 5.7-1.8"/>',
  file: FILE,
  'file-archive': FILE + '<path d="M11 4h2m-2 2h2m-2 2h2m-2 2h2m-2 2h2v3h-2z"/>',
  'file-audio': '<path d="M10 17V5l8-2v12M10 8l8-2"/><ellipse cx="7" cy="18" rx="3" ry="2"/><ellipse cx="15" cy="16" rx="3" ry="2"/>',
  'file-code': FILE + '<path d="m10 11-2 2 2 2m4-4 2 2-2 2m-1-5-2 7"/>',
  'file-document': FILE + '<path d="M8 10h8M8 13h8M8 16h6M8 18h4"/>',
  'file-image': '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8" cy="9" r="1.5"/><path d="m4 18 5-5 3 3 3-4 5 6"/>',
  'file-lines': '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/><path d="M9 13h6M9 17h6"/>',
  'file-pdf': FILE + '<path d="M8 16c2-2 3-5 3-7 1 4 3 6 5 6-3 0-5 1-8 1z"/>',
  'file-presentation': FILE + '<path d="M8 10h8v6H8zM12 16v2m-2 0h4"/>',
  'file-spreadsheet': FILE + '<path d="M8 11h8M8 14h8M8 17h8M12 11v6"/>',
  'file-text': FILE + '<path d="M8 11h8M8 14h8M8 17h5"/>',
  'file-video': '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m10 9 5 3-5 3z"/>',
  flame: '<path d="M12 3c1 3 5 5 5 10a5 5 0 0 1-10 0c0-2 1-3 2-4 0 2 1 3 2 3 0-3-1-6 1-9z"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  gauge: '<path d="M12 14l3.5-3.5"/><path d="M3.5 18a9 9 0 1 1 17 0"/>',
  gear: '<path d="M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4Z"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.55-1.03H3a2 2 0 1 1 0-4h.09a1.7 1.7 0 0 0 1.55-1.1 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34H9a1.7 1.7 0 0 0 1.03-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.55 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87V9a1.7 1.7 0 0 0 1.55 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.55 1.03Z"/>',
  'gear-simple': '<circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3 1a7 7 0 0 0-2-1.2L14.2 3h-4.4l-.4 2.7a7 7 0 0 0-2 1.2l-2.3-1-2 3.4 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.3-1a7 7 0 0 0 2 1.2l.4 2.7h4.4l.4-2.7a7 7 0 0 0 2-1.2l2.3 1 2-3.4-2-1.5c.1-.4.1-.8.1-1.2Z"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.5 3.8 5.6 3.8 9s-1.3 6.5-3.8 9c-2.5-2.5-3.8-5.6-3.8-9S9.5 5.5 12 3z"/>',
  goal: '<path d="M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z"/><path d="M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10z"/><path d="M12 11a1 1 0 1 0 0 2 1 1 0 0 0 0-2z"/>',
  grip: '<path d="M9 7h.01M15 7h.01M9 12h.01M15 12h.01M9 17h.01M15 17h.01"/>',
  group: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
  layers: '<rect x="3" y="9" width="12" height="11" rx="2"/><rect x="9" y="4" width="12" height="11" rx="2"/>',
  link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  'list-bullets': '<path d="M8 6h12M8 12h12M8 18h12"/><path d="M4 6h.01M4 12h.01M4 18h.01"/>',
  lock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
  menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
  mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0"/><path d="M12 17.5V21"/>',
  'mic-round': '<rect x="8.5" y="3" width="7" height="11" rx="3.5"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
  'mic-wide': '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0"/><path d="M12 18v3"/>',
  moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
  'org-chart': '<circle cx="12" cy="5" r="2.2"/><circle cx="5" cy="19" r="2.2"/><circle cx="19" cy="19" r="2.2"/><path d="M12 7.2V12M12 12H5v4.8M12 12h7v4.8"/>',
  paperclip: '<path d="M21 11.5 12.5 20a5.5 5.5 0 0 1-7.8-7.8l8.6-8.6a3.7 3.7 0 0 1 5.2 5.2l-8.5 8.5a1.8 1.8 0 0 1-2.6-2.6l7.8-7.8"/>',
  pin: '<path d="M12 17v5"/><path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9a2 2 0 0 1-1.1-1.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>',
  play: '<path d="M7 5v14l12-7z"/>',
  repeat: '<path d="M3 12a9 9 0 0 1 15.5-6.2L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-15.5 6.2L3 16"/><path d="M3 21v-5h5"/>',
  'shield-check': '<path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6z"/><path d="m9 12 2 2 4-4"/>',
  'shield-free': '<path d="M12 3 4.5 6v5.5c0 4.6 3.2 8.2 7.5 9.5 4.3-1.3 7.5-4.9 7.5-9.5V6z"/><path d="m9 12 2 2 4-4"/>',
  sidebar: '<rect x="3.5" y="4.5" width="17" height="15" rx="4"/><path d="M9.5 4.5v15"/>',
  sliders: '<path d="M4 7h10M18 7h2M4 17h2M10 17h10"/><circle cx="16" cy="7" r="2"/><circle cx="8" cy="17" r="2"/>',
  speak: '<path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z"/><path d="M15.5 9a4 4 0 0 1 0 6"/><path d="M18 6.5a7.5 7.5 0 0 1 0 11"/>',
  'speak-alt': '<path d="M4 9.5h3l4.5-4v13L7 14.5H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"/>',
  'split-col': '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M3.5 12h17"/>',
  'split-flip': '<path d="M7 4 3.5 7.5 7 11M3.5 7.5h13M17 13l3.5 3.5L17 20M20.5 16.5h-13"/>',
  'split-row': '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M12 4.5v15"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M4.6 4.6l1.4 1.4M18 18l1.4 1.4M2.5 12h2M19.5 12h2M4.6 19.4 6 18M18 6l1.4-1.4"/>',
  todo: '<rect x="4" y="4" width="16" height="16" rx="4"/><path d="m9 12 2 2 4-4"/>',
  tool: '<path d="M14.7 6.3a4 4 0 0 0-5 5L4 17v3h3l5.7-5.7a4 4 0 0 0 5-5l-2.6 2.6-2.4-.6-.6-2.4z"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  'user-round': '<path d="M20 21a8 8 0 0 0-16 0"/><path d="M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/>',
  users: '<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M15.5 4.8a3.5 3.5 0 0 1 0 6.4"/><path d="M18 14.3a6.5 6.5 0 0 1 3.5 5.7"/>',
  'window-bar': '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18"/>',
  'window-out': '<rect x="8" y="8" width="13" height="12" rx="2.5"/><path d="M16 4H5.5A2.5 2.5 0 0 0 3 6.5V16"/>',
  workflow: '<path d="M6 4h4v4H6zM14 16h4v4h-4zM14 4h4v4h-4z"/><path d="M10 6h4M8 8v5a3 3 0 0 0 3 3h3"/>',

  // Мессенджеры в шапке бота (линейные значки вместо круглой плашки)
  'msg-max': '<path d="M5 18V6l7 8 7-8v12"/>',
  'msg-telegram': '<path d="M20.5 4.5 3.8 11l5.2 1.9 1.9 5.6 2.7-3.4 4.4 3.2 2.5-13.8z"/><path d="m9 12.9 8.6-5.7"/>',
  'msg-whatsapp': '<path d="M4 20l1.2-4A8 8 0 1 1 8 18.8z"/><path d="M9.2 8.8c0 3 2.8 5.8 5.8 5.8l1-1.6-2-1-.9.8a4 4 0 0 1-1.7-1.7l.8-.9-1-2z"/>',

  // Значки провайдеров: свои, строки зашиты в сборку (логотипы остальных — providerIcons.js)
  'provider-cli': '<rect x="3.5" y="4.5" width="17" height="15" rx="3.5"/><path d="m8 10 3 2.5L8 15M13 15h3.5"/>',
  'provider-deloist': '<rect x="3.5" y="3.5" width="17" height="17" rx="5"/><path d="m8.5 12.4 2.4 2.4 4.6-5"/>',
  'provider-demo': '<path d="M12 3.5v4M12 16.5v4M3.5 12h4M16.5 12h4"/><path d="m12 8.5 1.2 2.3 2.3 1.2-2.3 1.2L12 15.5l-1.2-2.3L8.5 12l2.3-1.2z"/>',

  // Значки привычек: имена из src/habits.rs (ICON_NAMES); moon и check берутся из общих значков выше
  'habit-apple': '<path d="M12 8c-2-1.5-6-1-6 3.5 0 4 2.5 8.5 5 8.5.7 0 .9-.4 1-.4s.3.4 1 .4c2.5 0 5-4.5 5-8.5C18 7 14 6.5 12 8zM12 8c0-2 1-3.5 3-4"/>',
  'habit-bed': '<path d="M3 18V6M3 14h18v4M21 14v-2a3 3 0 0 0-3-3h-7v5"/><circle cx="7" cy="11" r="1.5"/>',
  'habit-book': '<path d="M12 6c-1.5-1.3-3.5-2-6-2v14c2.5 0 4.5.7 6 2 1.5-1.3 3.5-2 6-2V4c-2.5 0-4.5.7-6 2zM12 6v14"/>',
  'habit-brain': '<path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 1V5a2 2 0 0 0-3-1zM15 4a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 1"/>',
  'habit-coffee': '<path d="M4 9h12v5a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5zM16 10h2a2.5 2.5 0 0 1 0 5h-2M8 3v3M12 3v3"/>',
  'habit-drop': '<path d="M12 3s6 6.2 6 10.5a6 6 0 0 1-12 0C6 9.2 12 3 12 3z"/>',
  'habit-dumbbell': '<path d="M6.5 6.5v11M3.5 9v6M17.5 6.5v11M20.5 9v6M6.5 12h11"/>',
  'habit-heart': '<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.5A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>',
  'habit-home': '<path d="M4 11l8-7 8 7M6 10v10h12V10M10 20v-6h4v6"/>',
  'habit-leaf': '<path d="M5 19C5 10 10 5 20 4c0 10-5 15-14 15zM5 19l8-8"/>',
  'habit-music': '<path d="M9 18V6l10-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/>',
  'habit-pen': '<path d="M4 20l1-4L16.5 4.5a2.1 2.1 0 0 1 3 3L8 19zM14.5 6.5l3 3"/>',
  'habit-phone': '<rect x="7" y="3" width="10" height="18" rx="2.5"/><path d="M11 18h2"/>',
  'habit-run': '<circle cx="14" cy="5" r="2"/><path d="M5 20l3.5-5 3 1.5 2-4.5-3-2-3 2M11.5 16.5L13 20M14 11l3.5 1.5 2.5-1"/>',
  'habit-star': '<path d="M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.9L12 17l-5.2 2.7 1-5.9L3.5 9.7l5.9-.8z"/>',
  'habit-sun': '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',

  // Рабочие процессы: значки узлов, панели инструментов и меню холста (имена — как в WfIcon)
  'wf-approval': '<path d="M12 3 4.5 6v5.5c0 4.2 3 7.6 7.5 9.5 4.5-1.9 7.5-5.3 7.5-9.5V6z"/><path d="m8.8 12 2.4 2.4 4.2-4.6"/>',
  'wf-asset': '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/><path d="M9 13h6M9 17h4"/>',
  'wf-back': '<path d="M4 4h12v12H4z"/><path d="M8 20h12V8"/>',
  'wf-bell': '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20a2 2 0 0 0 4 0"/>',
  'wf-bot': '<path d="M12 3v3"/><path d="M7 8h10a3 3 0 0 1 3 3v5a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3v-5a3 3 0 0 1 3-3z"/><path d="M9 13v1M15 13v1"/>',
  'wf-calendar': '<path d="M4 6h16v14H4z"/><path d="M4 10h16"/><path d="M8 3v4M16 3v4"/>',
  'wf-chat': '<path d="M4 5h16v11H9l-5 4z"/><path d="M8 9.5h8M8 12.5h5"/>',
  'wf-chevron': '<path d="m7 10 5 5 5-5"/>',
  'wf-clock': '<path d="M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z"/><path d="M12 8v4.5l3 1.5"/>',
  'wf-condition': '<path d="M6 4v9"/><path d="M6 20a2 2 0 1 0 .01 0"/><path d="M18 8a2 2 0 1 0 .01 0"/><path d="M18 10c0 5-12 2-12 8"/>',
  'wf-copy': '<path d="M9 9h10v11H9z"/><path d="M5 15V4h10"/>',
  'wf-discord': '<path d="M7 7c3-1.4 7-1.4 10 0 2 3 3 6 3 10-1.5 1-3 1.6-4.5 2l-1-2c-1.5.4-2.5.4-4 0l-1 2C6 18.6 5 18 4 17c0-4 1-7 3-10z"/><path d="M9.5 12.5a.5.5 0 1 0 .01 0"/><path d="M14.5 12.5a.5.5 0 1 0 .01 0"/>',
  'wf-dup': '<path d="M8 8h12v12H8z"/><path d="M4 16V4h12"/>',
  'wf-eraser': '<path d="M7 20h12"/><path d="M5 15l8-9 6 6-6 8H8z"/><path d="M10 10l6 6"/>',
  'wf-fit': '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
  'wf-hand': '<path d="M8 12V6.5a1.5 1.5 0 0 1 3 0V11"/><path d="M11 10V4.5a1.5 1.5 0 0 1 3 0V11"/><path d="M14 10V6a1.5 1.5 0 0 1 3 0v9a6 6 0 0 1-6 6h-1a6 6 0 0 1-4.5-2L3 14.5a1.6 1.6 0 0 1 2.4-2L8 15"/>',
  'wf-history': '<path d="M4 12a8 8 0 1 0 2.5-5.8"/><path d="M4 4v5h5"/><path d="M12 8v4.5l3 1.5"/>',
  'wf-image': '<path d="M4 5h16v14H4z"/><path d="M4 16l5-5 4 4 3-3 4 4"/><path d="M15 9.5a1 1 0 1 0 .01 0"/>',
  'wf-link': '<path d="M4 12h5l3-6 3 12 3-6h2"/>',
  'wf-list': '<path d="M4 6h16M4 12h16M4 18h16"/>',
  'wf-llm': '<path d="M12 3l1.8 4.7L18.5 9.5 13.8 11.3 12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18.5 15.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z"/>',
  'wf-lock': '<path d="M6 11h12v9H6z"/><path d="M8.5 11V8a3.5 3.5 0 0 1 7 0v3"/>',
  'wf-manual': '<path d="M8 12V6.5a1.5 1.5 0 0 1 3 0V11"/><path d="M11 10.5a1.5 1.5 0 0 1 3 0V12"/><path d="M14 11.5a1.5 1.5 0 0 1 3 0V15a6 6 0 0 1-6 6h-.5A6 6 0 0 1 6 18l-2.5-4.5a1.6 1.6 0 0 1 2.4-2L8 14"/>',
  'wf-map': '<path d="M9 4 3 6.5v13.5l6-2.5 6 2.5 6-2.5V4l-6 2.5z"/><path d="M9 4v13.5M15 6.5V20"/>',
  'wf-marker': '<path d="M5 19l3-1 8-8-2-2-8 8z"/><path d="M14 8l2-2 2 2-2 2"/><path d="M4 21h8"/>',
  'wf-minus': '<path d="M5 12h14"/>',
  'wf-more': '<path d="M6 12h.01M12 12h.01M18 12h.01"/>',
  'wf-note': '<path d="M5 4h14v10l-5 6H5z"/><path d="M14 20v-6h5"/>',
  'wf-output': '<path d="M21 3 10 14"/><path d="M21 3l-7 18-4-7-7-4z"/>',
  'wf-paste': '<path d="M9 4h6v3H9z"/><path d="M7 5H5v15h14V5h-2"/>',
  'wf-pause': '<path d="M8 5v14M16 5v14"/>',
  'wf-pencil': '<path d="M4 20l1-4L16 5l3 3L8 19z"/><path d="M14 7l3 3"/>',
  'wf-play': '<path d="M7 4.5v15l12-7.5z"/>',
  'wf-prompt': '<path d="M4 6h16M4 12h16M4 18h10"/>',
  'wf-redo': '<path d="m15 14 5-5-5-5"/><path d="M20 9h-9a7 7 0 0 0 0 14h2"/>',
  'wf-retry': '<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>',
  'wf-run-step': '<path d="M7 4.5v15l12-7.5z"/><path d="M3 4v16"/>',
  'wf-scissors': '<path d="M6 7a2.5 2.5 0 1 0 0 .01"/><path d="M6 17a2.5 2.5 0 1 0 0 .01"/><path d="M8 8.5 20 18M8 15.5 20 6"/>',
  'wf-search': '<path d="M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z"/><path d="m20 20-4-4"/>',
  'wf-select': '<path d="m5 3 14 7-6 2-2 6z"/>',
  'wf-select-all': '<path d="M4 4h16v16H4z"/><path d="M8 8h8v8H8z"/>',
  'wf-shape': '<path d="M4 4h8v8H4z"/><path d="M16 21a4 4 0 1 0 0-8 4 4 0 0 0 0 8z"/>',
  'wf-skip': '<path d="M5 12h14"/><path d="m14 7 5 5-5 5"/>',
  'wf-slack': '<path d="M9 3v7"/><path d="M15 14v7"/><path d="M3 15h7"/><path d="M14 9h7"/><path d="M6 3a1.5 1.5 0 0 0 0 3h3V3z"/><path d="M18 21a1.5 1.5 0 0 0 0-3h-3v3z"/><path d="M3 18a1.5 1.5 0 0 0 3 0v-3H3z"/><path d="M21 6a1.5 1.5 0 0 0-3 0v3h3z"/>',
  'wf-soon': '<path d="M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z"/><path d="M12 8v4l2.5 1.5"/>',
  'wf-stop': '<path d="M6 6h12v12H6z"/>',
  'wf-telegram': '<path d="M21 4 3 11l5 2 2 6 3-4 5 4z"/><path d="m8 13 9-6"/>',
  'wf-text': '<path d="M5 6V4h14v2"/><path d="M12 4v16"/><path d="M9 20h6"/>',
  'wf-trash': '<path d="M5 7h14M10 4h4M7 7l1 13h8l1-13"/><path d="M10 11v6M14 11v6"/>',
  'wf-trigger': '<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>',
  'wf-undo': '<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 0 1 0 14h-2"/>',
  'wf-unlock': '<path d="M6 11h12v9H6z"/><path d="M8.5 11V8a3.5 3.5 0 0 1 6.8-1"/>',
  'wf-versions': '<path d="M12 3l9 5-9 5-9-5z"/><path d="M3 13l9 5 9-5"/>',
  'wf-webhook': '<path d="M10 15a4 4 0 1 1 5.5-3.7"/><path d="M8 15h8"/><path d="M16 15a2.2 2.2 0 1 1 0 .01"/><path d="M6.5 18a2.2 2.2 0 1 1 0 .01"/><path d="M9 11.5 6.5 16"/>',
};

const escapeAttr = (value) => String(value).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

// Готовый <svg> строкой: для страниц без Svelte и для разметки, которую собирает код.
// size не задан — размер берёт CSS (как у значков с классом i). Неизвестное имя — ошибка, не пустая иконка.
export function icon(name, { size, stroke = 2, className = '', label = '' } = {}) {
  if (!Object.hasOwn(ICONS, name)) throw new Error(`unknown icon: ${name}`);
  const cls = className ? ` class="${escapeAttr(className)}"` : '';
  const dim = size ? ` width="${size}" height="${size}"` : '';
  const a11y = label ? ` role="img" aria-label="${escapeAttr(label)}"` : ' aria-hidden="true"';
  return `<svg${cls} viewBox="0 0 24 24"${dim} fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round"${a11y}>${ICONS[name]}</svg>`;
}

// Статичная разметка страниц (web/*.html): <svg class="i" viewBox=… stroke-width="2.2" data-icon="имя"></svg> с атрибутами кита
// (они же держат вид до запуска скриптов). При старте страница зовёт paintIcons(): содержимое берётся из ICONS,
// недостающие атрибуты кита дописываются.
const KIT_ATTRS = {
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  'stroke-width': '2',
  'stroke-linecap': 'round',
  'stroke-linejoin': 'round',
  'aria-hidden': 'true',
};

export function paintIcons(root = document) {
  for (const el of root.querySelectorAll('svg[data-icon]')) {
    const name = el.getAttribute('data-icon');
    if (!Object.hasOwn(ICONS, name)) {
      console.warn(`unknown icon: ${name}`);
      continue;
    }
    for (const [attr, value] of Object.entries(KIT_ATTRS)) if (!el.hasAttribute(attr)) el.setAttribute(attr, value);
    el.innerHTML = ICONS[name];
  }
}
