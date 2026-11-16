# Соглашения проекта Deloist Боты

Полная версия лежит в репозитории: `deloist-agent/docs/ui-svelte.md` (эта справка из неё и дополняет её).

## Раскладка

```
deloist-agent/
├── ui/                        Node-проект (только для правок интерфейса)
│   ├── package.json           версии svelte, vite, plugin точные; .npmrc: save-exact
│   ├── svelte.config.js       css: 'external', runes: true
│   ├── vite.config.js         IIFE-библиотека → ../web/ui/kit.js + kit.css
│   └── src/
│       ├── index.js           window.DeloistUI = { mountLimitRing, … }
│       ├── components/*.svelte
│       └── lib/{i18n.js, mount.svelte.js}
├── web/ui/                    СОБРАННОЕ, коммитится: kit.js, kit.css, .source-hash
├── web/ui-demo.html / .js     витрина, только debug-сборка
├── tools/check_ui_built.mjs   хэш исходников = web/ui/.source-hash
└── tools/check_i18n.mjs       сканирует и ui/src/**/*.svelte
```

## Команды

```bash
cd ui && npm ci && npm run build     # пересобрать web/ui и хэш
node tools/check_ui_built.mjs        # из корня репозитория
node tools/check_i18n.mjs
cargo test --test surface ui_kit::
```

`cargo build`/`cargo test` Node не требуют: `build.rs` встраивает все файлы `web/ui/**` (кроме начинающихся с точки), `server.rs` отдаёт их как `/ui/<файл>` за сессией окна. Новый файл правок Rust не требует.

## Монтирование в старую страницу

```html
<link rel="stylesheet" href="/ui/kit.css" />
<script src="/i18n.js"></script>      <!-- до kit.js: компоненты зовут window.tr -->
<script src="/ui/kit.js"></script>
```
```js
const ring = DeloistUI.mountLimitRing(el, { value: 0.4, color: '#22c55e', size: 56, label: 'Аня' });
ring.update({ value: 0.5 });  ring.destroy();
```
Новый компонент: `ui/src/components/Name.svelte`, `mountName` в `ui/src/index.js` через `mountComponent`, сборка, коммит `web/ui` вместе с исходниками. CSP запрещает встроенные `<script>`; код страницы — отдельный `.js`.

## Перевод

Видимый текст только `tr('русский ключ', { name })` из `../lib/i18n.js` (обёртка над `window.tr`). Ключ — статичная строка, без `${}` внутри; значения через vars `{name}`. Ключ добавить в EN в `web/i18n.js` (плейсхолдеры совпадают). Русский текст вне `tr()` в `.svelte` — ошибка `check_i18n`.

## Стили и токены

Цвета/радиусы/отступы из CSS-переменных `web/kit.css` (готовит сессия design-kit): `var(--text)`, `var(--surface)`… Пока токена нет — `var(--токен, запасное значение)`. Стили в `<style>` компонента (область видимости Svelte), CSS идёт во внешний `kit.css`. Общие токены в компонентах не определять.

## Лёгкость

Нет `motion`, GSAP, Lottie, целых иконочных наборов. Иконки — inline SVG. Нет бесконечных анимаций, таймеров и rAF в покое; transition короткие, с `prefers-reduced-motion`. Бюджет: `kit.js` вокруг 15 КБ gzip на старте (рантайм Svelte ≈14 КБ), каждый компонент — килобайты.

## Правило переезда

Старый экран (`index.html`, `app.js`, `dock.*`) переезжает на Svelte **только когда его переделывают**. Не переписывать работающее ради Svelte. Новые экраны и редизайны — компонентами.

## Пакеты

Только внутри `ui/`, из npm, версия точная. Обновляя: `npm view <pkg> version`, ставить с `--save-exact`, коммитить `package-lock.json`, обновить версии в `docs/ui-svelte.md` и в `SKILL.md`.

## Известные грабли

- Рунам вне `.svelte` нужен суффикс `.svelte.js`.
- `mount` не запускает эффекты сразу (`flushSync()` в тестах).
- Экспортировать переприсваиваемый `$state` из модуля нельзя (см. imperative-api-and-state.md).
- Артефакт сборки зависит от версий пакетов: после `npm install` пересобрать и закоммитить `web/ui`.
- Хэш считается по нормализованным переводам строк, на CRLF в Windows не ломается.
