---
name: svelte
description: Svelte 5 (runes) для веб-интерфейса Deloist Боты — папка ui/, файлы *.svelte, web/ui/, компоненты, док ботов, интерфейс бота. Use for ANY work on ui/, *.svelte, *.svelte.js, web/ui/ (kit.js, kit.css), window.DeloistUI, LimitRing, "компонент", "Svelte", "док", "интерфейс бота", Vite build of the UI. Не даёт писать синтаксис Svelte 4 ($:, export let, on:click, slots, createEventDispatcher, new App()), знает сборку, тесты, перевод tr(), токены kit.css и правило «старый экран переезжает только при редизайне».
---

# Svelte 5 в Deloist Боты

Версия в проекте: **svelte 5.57.3**, vite 8.3.8, @sveltejs/vite-plugin-svelte 7.3.1 (закреплены точно). Справочник снят с официальных доков 2026-11-11 (svelte.dev/llms.txt, `docs/svelte/llms.txt`). Перед работой сверь версии с `ui/package.json`: если выросли, доки могли измениться.

**Всегда Svelte 5, режим runes.** В `ui/svelte.config.js` стоит `runes: true`: синтаксис Svelte 4 просто не соберётся. Не пиши `$:`, `export let`, `on:click`, `<slot>`, `createEventDispatcher`, `new Component()`, `$store` для нового состояния. Таблица замен: [references/svelte4-to-5.md](references/svelte4-to-5.md).

## Минимум, который надо помнить

```svelte
<script>
  import { tr } from '../lib/i18n.js';            // перевод: только через tr()
  let { value = 0, label = '', onchange, children } = $props();   // вход
  let open = $state(false);                       // состояние
  const pct = $derived(Math.round(value * 100));  // вычисляемое (чистое, без побочек)
  $effect(() => { /* побочный эффект: DOM, таймер */ return () => { /* уборка */ }; });
</script>
<button onclick={() => { open = !open; onchange?.(open); }}>{tr('Открыть')}: {pct}%</button>
{#if open}{@render children?.()}{/if}
```

- `$state` — реактивная переменная (объекты и массивы глубоко реактивны); большие данные только на чтение — `$state.raw`.
- `$derived(...)` / `$derived.by(() => ...)` — вычисления; `$effect` не для синхронизации состояния, только для побочных эффектов, и с уборкой.
- `$props()` — входы, значения по умолчанию через деструктуризацию; `$bindable()` только если действительно нужна двусторонняя связь.
- События: атрибуты `onclick={...}`; родителю сообщаем **колбэк-пропсом** (`onchange`), не событием.
- Содержимое: `children` и `{#snippet}` + `{@render}` вместо слотов.
- Руны не импортируются; `$state` и т. п. в `.js` работают только в файлах `*.svelte.js`.

Подробности: [runes](references/runes.md), [шаблоны, события, snippets](references/templates-events-snippets.md), [mount/unmount, `.svelte.js`, stores](references/imperative-api-and-state.md), [transitions и стоимость CPU](references/transitions-and-cpu.md).

## Наши соглашения (коротко, полно — [references/project-conventions.md](references/project-conventions.md) и `docs/ui-svelte.md`)

1. Исходники в `ui/src/components/*.svelte`, точка входа `ui/src/index.js` вешает `window.DeloistUI.mountX(el, props)`. Старые страницы подключают `/ui/kit.css`, `/i18n.js`, `/ui/kit.js` обычными тегами.
2. **Собранный `web/ui/` коммитится.** После правки в `ui/`: `cd ui && npm run build`, затем из корня `node tools/check_ui_built.mjs` и `node tools/check_i18n.mjs`. Пока не пройдёт — не коммитить.
3. Текст только через `tr('русский ключ', {vars})`; ключ добавить в EN в `web/i18n.js`.
4. Цвета, радиусы, отступы — токены из `web/kit.css` (`var(--токен, запас)`).
5. Никаких тяжёлых библиотек (`motion`, целые иконочные пакеты); никаких бесконечных анимаций и опроса в покое; `prefers-reduced-motion` учитывать.
6. **Старый экран переезжает на Svelte только когда его переделывают (редизайн).** Не переписывать работающее ради Svelte.
7. CSP запрещает встроенные `<script>`: код всегда отдельным файлом.
8. Установка пакетов только в `ui/`, версия точная; обновляя, брать `npm view <pkg> version` и записывать версии в `docs/ui-svelte.md` и сюда.

## Перед сдачей

`npm run build`, `check_ui_built`, `check_i18n`, `cargo test --test surface ui_kit::`; страницу проверить в debug-сборке на `/ui-demo.html` (`--server`): `document.getAnimations().length === 0` в покое, нет мутаций DOM. Размер `kit.js` смотреть в выводе сборки (было ≈14,6 КБ gzip на одном компоненте, почти всё — рантайм).
