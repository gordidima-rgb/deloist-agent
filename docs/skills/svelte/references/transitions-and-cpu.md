# Transitions, анимации и стоимость CPU

Приложение чувствительно к CPU и памяти: один бесконечный CSS-спиннер однажды съел 33 % ядра. Покой должен быть ~0.

## Встроенное

- `transition:fade|fly|slide|scale|blur|draw|crossfade` (`svelte/transition`), `in:` / `out:` отдельно. С Svelte 5 transitions **локальные** по умолчанию (играют, только когда меняется свой блок, а не родительский); `|global` — если нужно иначе.
- `animate:flip` (`svelte/animate`) — для перестановок в keyed `{#each}`.
- `svelte/motion`: классы `Spring` и `Tween` (замена stores `spring()`/`tweened()`), `prefersReducedMotion` (5.7+): `prefersReducedMotion.current`.
- `svelte/easing` — функции плавности.
- Свой transition — функция, возвращающая `{ duration, css: (t, u) => '...' }`; предпочитать `css`, а не `tick` (css идёт на композиторе без JS на кадр).

## Правила проекта

1. **Никаких бесконечных анимаций** (`animation-iteration-count: infinite`, `repeat`, rAF-циклы в покое, `setInterval` для «дыхания»). Живое состояние показывать статикой или одноразовой короткой анимацией.
2. Все transition/анимации — короткие (≤ 300 мс) и выключаются `@media (prefers-reduced-motion: reduce) { … transition: none }`. В JS-параметрах учитывать `prefersReducedMotion.current`.
3. Анимировать только `transform` и `opacity` (композитор). Не `width/height/top/left/box-shadow/filter` на большом числе элементов.
4. `Spring`/`Tween` создавать только там, где значение реально плавно меняется; для статичных — не нужны (внутри rAF, пока идёт движение).
5. `$effect` с `setInterval`/`setTimeout`/подписками — только с уборкой; в покое ничего не опрашивать, обновления приходят событиями сервера (SSE), а не таймером.
6. Списки: `{#each items as it (it.id)}` с ключом; большие неизменяемые данные — `$state.raw`; не заводить `$derived` на каждую мелочь, если значение дешёвое и нужно в одном месте.
7. Не тянуть `motion`, GSAP, Lottie, иконочные пакеты целиком. Иконка — inline SVG в компоненте.
8. `will-change`/слои — только на время анимации, не навсегда.
9. Один `mount` на узел; при удалении узла со страницы вызывать `destroy()`, иначе эффекты и подписки утекут.

## Как проверить «ноль в покое»

В debug-сборке на `/ui-demo.html`: `document.getAnimations().length` должен быть 0; `MutationObserver` на документ за 5 с — 0 записей; `PerformanceObserver` без long tasks. Процессорную нагрузку сравнивать A/B (страница с компонентом и пустая) — фон других программ шумит на несколько процентов.
