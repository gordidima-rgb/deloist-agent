# mount / unmount / hydrate, `.svelte.js`, stores

Компоненты в Svelte 5 — функции, а не классы. `new App({target})` больше нет.

## Императивный API (`import { mount, unmount, hydrate, flushSync, tick } from 'svelte'`)

```js
const app = mount(Component, { target: element, props: { value: 0 } });   // монтирование
await unmount(app, { outro: true });   // outro: проиграть transition перед удалением; Promise
```
- `mount` можно вызывать много раз на странице и изнутри приложения (подсказки, оверлеи).
- Эффекты (`$effect`, `onMount`, actions) **не** запускаются синхронно внутри `mount`/`hydrate`; нужно сразу выполнить — `flushSync()`. Для обновлений ждать DOM — `await tick()`.
- `hydrate(Component, {target, props})` — как `mount`, но переиспользует HTML от SSR (`render` из `svelte/server`). **У нас SSR нет**, `hydrate` и `render` не нужны.
- Живые пропсы снаружи: создать реактивный объект `const props = $state({...})` (только в `.svelte.js`!), передать его как `props` в `mount`, затем `Object.assign(props, next)`. Так сделан `ui/src/lib/mount.svelte.js` (`mountComponent` → `{ update, destroy }`). Пропсы, не переданные при создании, не появятся позже: все ключи объявлять сразу.

## `.svelte.js` / `.svelte.ts`

Обычные модули, в которых работают руны. Для переиспользуемой логики и общего состояния.

```js
// counter.svelte.js
export function createCounter() {
  let count = $state(0);
  return { get count() { return count; }, inc: () => count++ };
}
export const shared = $state({ name: '' });   // экспортировать можно объект, который мутируют
```
**Нельзя** `export let count = $state(0)` с последующим переприсваиванием (`count += 1`): компилятор работает по одному файлу, импортёры не обернут чтение в `$.get`. Выходы: экспортировать объект и менять его поля, или экспортировать геттер/функции, или класс с `$state`-полями.

## Stores (`svelte/store`) против рун

Stores (`writable`, `readable`, `derived`, `$store` в компоненте) остались и работают, но в Svelte 5 нужны мало. Для нового кода — руны в `.svelte.js`. Store оправдан только для контракта подписки с чужим кодом или сложных асинхронных потоков (RxJS-стиль). Не смешивать в одном компоненте `$store` и руны без причины. `$`-префикс у локальных переменных запрещён.

## Реактивные встроенные классы (`svelte/reactivity`)

`SvelteMap`, `SvelteSet`, `SvelteDate`, `SvelteURL`, `SvelteURLSearchParams`, `MediaQuery`, `createSubscriber`: обычные коллекции не реактивны, этим заменять, если нужна реактивность по ключу.

## Контекст

`setContext(key, value)` / `getContext(key)` / `createContext()` — только во время инициализации компонента. Для состояния — класть в контекст объект с `$state`. При `mount` вручную контекст родителя не наследуется: передавать `context: new Map([...])` в опциях.
