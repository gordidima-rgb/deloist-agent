<script module>
  // Выбранный раздел живёт до перезагрузки страницы, а не в storage:
  // закрытое и снова открытое окно настроек остаётся на том же разделе.
  let selectedSection = $state('main');
  // Обработчик смонтированной навигации: выбор снаружи показывает раздел так же, как клик.
  let notify = null;

  export function select(sectionId) {
    if (!sectionId) return;
    selectedSection = sectionId;
    notify?.(sectionId);
  }

  export function current() {
    return selectedSection;
  }
</script>

<script>
  // Навигация настроек: строки кита. Стрелки ведут фокус, Enter выбирает раздел.
  import ListItem from '../kit/ListItem.svelte';
  import { tr } from '../../lib/i18n.js';

  let { sections = [], onselect } = $props();
  let root = $state();

  $effect(() => {
    notify = onselect ?? null;
    return () => { if (notify === onselect) notify = null; };
  });

  const buttons = () => [...(root?.querySelectorAll('.k-list-item__main') ?? [])];

  // Делегирование с списка: у строк кита кнопка внутри, а слушатель на неинтерактивном
  // <ul> Svelte пометил бы предупреждением.
  $effect(() => {
    if (!root) return;
    root.addEventListener('keydown', keydown);
    return () => root.removeEventListener('keydown', keydown);
  });

  function pick(sectionId) {
    if (sectionId === selectedSection) return;
    selectedSection = sectionId;
    onselect?.(sectionId);
  }

  function keydown(event) {
    if (!sections.length) return;
    const button = event.target.closest?.('.k-list-item__main');
    const at = button ? buttons().indexOf(button) : -1;
    if (event.key === 'Enter' || event.key === ' ') {
      if (at < 0) return;
      event.preventDefault();
      pick(sections[at].id);
      return;
    }
    const step =
      event.key === 'ArrowDown' || event.key === 'ArrowRight'
        ? 1
        : event.key === 'ArrowUp' || event.key === 'ArrowLeft'
          ? -1
          : 0;
    if (!step) return;
    event.preventDefault();
    const selected = Math.max(0, sections.findIndex((section) => section.id === selectedSection));
    const next = at < 0 ? (selected + step + sections.length) % sections.length : (at + step + sections.length) % sections.length;
    buttons()[next]?.focus();
  }
</script>

<ul class="k-list settings-nav__list" role="list" aria-label={tr('Разделы настроек')} bind:this={root}>
  {#each sections as section (section.id)}
    <ListItem
      title={section.label}
      active={selectedSection === section.id}
      onclick={() => pick(section.id)}
      data-section={section.id}
    />
  {/each}
</ul>
