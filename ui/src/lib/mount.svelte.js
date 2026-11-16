import { mount, unmount } from 'svelte';

// Монтирует компонент в готовый DOM-узел старой страницы и возвращает ручку
// { update(props), destroy() }. Свойства живые: update() перерисовывает без пересоздания.
export function mountComponent(Component, target, initial = {}) {
  if (!(target instanceof Element)) throw new Error('DeloistUI: target должен быть элементом DOM');
  const props = $state({ ...initial });
  const instance = mount(Component, { target, props });
  return {
    update(next) {
      Object.assign(props, next);
    },
    destroy() {
      unmount(instance);
    },
  };
}
