# Шаблоны, события, snippets

Блоки `{#if}`, `{#each items as item (item.id)}`, `{#key}`, `{#await}` не изменились; в `{#each}` всегда давать ключ, если список меняется.

## События — обычные атрибуты

```svelte
<button onclick={() => count++}>…</button>
<button {onclick}>…</button>            <!-- сокращение, если функция называется onclick -->
<input oninput={(e) => (text = e.currentTarget.value)} />
```
- Двоеточия нет: `on:click` → `onclick`. Модификаторов (`|preventDefault`) нет: `e.preventDefault()` внутри обработчика. `capture` — `onclickcapture`.
- Делегирование событий сделано самим Svelte; для `window`/`document` — `<svelte:window onkeydown={…} />`, `<svelte:document>`.
- Компонент сообщает наружу **колбэк-пропсом**: `let { onchange } = $props(); onchange?.(v)`. `createEventDispatcher` устарел.
- Несколько обработчиков на один элемент — вызывать из одной функции (дублировать атрибут нельзя).

## Snippets вместо слотов

```svelte
<!-- Родитель -->
<Table data={rows}>
  {#snippet header()}<th>Имя</th>{/snippet}
  {#snippet row(d)}<td>{d.name}</td>{/snippet}
  Текст вне snippet попадает в `children`
</Table>

<!-- Table.svelte -->
<script>
  let { data, header, row, children } = $props();
</script>
<table>
  <thead><tr>{@render header()}</tr></thead>
  <tbody>{#each data as d (d.id)}<tr>{@render row(d)}</tr>{/each}</tbody>
</table>
{@render children?.()}
```
- Snippet — значение: объявляется где угодно в области видимости, передаётся пропсом, рисуется `{@render name(args)}`. Вне своей лексической области не виден.
- Параметры с умолчаниями и деструктуризацией можно, rest-параметров нет.
- Необязательный snippet: `{@render children?.()}`.
- Типы (TS): `import type { Snippet } from 'svelte'` → `Snippet`, `Snippet<[Row]>`. Мы пишем на чистом JS, JSDoc по желанию.

## Привязки

`bind:value`, `bind:checked`, `bind:group`, `bind:this={el}` (элемент или экземпляр компонента), `bind:prop` на компонент — только к `$bindable`. Экспорты компонента (`export function`) доступны через `bind:this` → `comp.fn()`, а не `bind:fn`.

## Прочее синтаксис

- `{@attach fn}` (5.29+) — замена `use:action`, перезапускается при смене зависимостей, возвращает уборку. `use:action` ещё работает.
- `<svelte:element this={tag}>`, `<svelte:boundary>` (`failed`/`pending` snippets, `onerror`), `<svelte:head>`, `<svelte:options>`.
- `<svelte:component this={C}>` не нужен: `<C />` с динамической переменной; `<svelte:self>` не нужен: импортируй сам файл.
- `{@const x = …}`, `{@html}` (только доверенное; с данными бота — никогда без очистки), `{@debug}`.
- Контекст: `setContext/getContext/createContext` в инициализации компонента; для разделяемого состояния — класс или `$state` в `.svelte.js`.
- Жизненный цикл: `onMount(() => { …; return cleanup })`, `onDestroy`, `tick()`. `beforeUpdate/afterUpdate` устарели → `$effect.pre` + `tick`.
- Доступ к rest-пропсам: `{...rest}`; `$$props`, `$$restProps` не использовать.
