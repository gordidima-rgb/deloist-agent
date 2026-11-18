<script>
  // Карточки файлов результата (.k-filecard): значок по типу, имя, папка и «Открыть». Сами файлы не читает:
  // нажатие отдаёт путь в onopen, а открывает его экран через «Файлы» бота.
  import Icon from './Icon.svelte';
  import Button from '../buttons/Button.svelte';
  import { fileType } from '../../../lib/fileType.js';
  import { tr } from '../../../lib/i18n.js';

  let { files = [], onopen, class: className = '', ...rest } = $props();

  const parts = (path) => {
    const list = String(path).replace(/\\/g, '/').split('/');
    return { name: list.pop(), dir: list.slice(-2).join('/') };
  };
  const iconOf = (path) => {
    const type = fileType(path);
    return type === 'unknown' ? 'file-text' : `file-${type}`;
  };
</script>

<ul class="k-filecards {className}" {...rest}>
  {#each files as path (path)}
    {@const { name, dir } = parts(path)}
    <li class="k-filecard" title={path}>
      <span class="k-filecard__ico"><Icon name={iconOf(path)} size={null} /></span>
      <span class="k-filecard__t"><b>{name}</b>{#if dir}<small>{dir}</small>{/if}</span>
      <Button size="sm" variant="ghost" onclick={() => onopen?.(path)}>{tr('Открыть')}</Button>
    </li>
  {/each}
</ul>
