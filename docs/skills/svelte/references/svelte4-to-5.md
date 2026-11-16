# Svelte 4 → 5: так больше не писать

В проекте включён `runes: true`: старый синтаксис в `ui/` не скомпилируется. Если пример из интернета выглядит как левая колонка — перепиши.

| Svelte 4 (не писать) | Svelte 5 (писать) |
| --- | --- |
| `let count = 0;` как реактивное | `let count = $state(0);` |
| `$: double = count * 2;` | `const double = $derived(count * 2);` |
| `$: { console.log(count) }` / `$: if (x) …` | `$effect(() => { console.log(count) })` (только побочки) |
| `export let foo = 1;` | `let { foo = 1 } = $props();` |
| `export let a; export let b;` + `$$restProps` | `let { a, b, ...rest } = $props();` |
| `$$props` | `let props = $props();` |
| любой проп bindable | `let { v = $bindable() } = $props();` и `bind:v` |
| `on:click={fn}` | `onclick={fn}` |
| `on:click\|preventDefault={fn}` | `onclick={(e) => { e.preventDefault(); fn(e); }}` |
| `on:click` без обработчика (проброс события) | `{onclick}` через `...rest` или явный колбэк-проп |
| `createEventDispatcher()` + `dispatch('x')` | колбэк-проп `let { onx } = $props(); onx?.(…)` |
| `<slot />` | `{@render children?.()}` |
| `<slot name="x" {a}>` | `{@render x?.(a)}`, проп `x` — snippet |
| `<p slot="x" let:y>` | `{#snippet x(y)}<p>…</p>{/snippet}` внутри тега компонента |
| `<svelte:component this={C} />` | `<C />` (C — переменная с заглавной буквы) |
| `<svelte:self />` | импорт самого файла |
| `use:action` (для нового) | `{@attach fn}` (хуже не будет и `use:`, но новое пиши attach) |
| `new App({ target, props })` | `const app = mount(App, { target, props })` |
| `app.$set(props)` / `app.$on` / `app.$destroy()` | `$state`-объект как `props` и `Object.assign`; колбэк-пропсы; `unmount(app)` |
| `bind:fn` на экспорт компонента | `bind:this={c}` → `c.fn()` |
| `beforeUpdate` / `afterUpdate` | `$effect.pre` / `$effect` + `tick()` |
| `import { writable } from 'svelte/store'` для нового состояния | `$state` в `.svelte.js` |
| `$store` автоподписка в новом коде | `$state` / геттеры |
| `spring()`, `tweened()` из `svelte/motion` | `new Spring(…)`, `new Tween(…)` |
| `class:foo={cond}` в сложных случаях | `class={{ foo: cond }}` |
| `import { run } from 'svelte/legacy'`, `preventDefault` из legacy | не использовать, это только для миграции |
| `export const foo` + `bind:foo` | экспорт функции и `bind:this` |
| `<svelte:options accessors />` | не работает в runes; экспорты + `bind:this` |
| `compilerOptions: { runes: false }` / смешанный режим | не включать |

## Прочие различия

- Компоненты — функции; `mount`/`unmount`/`hydrate` вместо `new`/`$destroy`.
- `mount` не запускает эффекты синхронно: `flushSync()`.
- Пробелы в шаблоне обрабатываются иначе (схлопываются агрессивнее).
- Нужен современный браузер (WebView2 — норм).
- `children` — зарезервированное имя пропса.
- Transitions по умолчанию локальные.
- Не нужно импортировать руны; не оборачивать их в функции; не присваивать переменным.
- Автомиграция (`npx sv migrate svelte-5`) у нас не нужна: переносим по экрану при редизайне, руками.
