<script>
  // Карточка рутины «Углей»: сверху строка «название + действия», под ней задание на всю ширину (3 строки с затуханием,
  // «Показать полностью» раскрывает с абзацами), ниже метки: бот, как часто, следующий запуск (оранжевым), прошлый запуск.
  // Выключенная рутина бледнеет, а не исчезает.
  import { tr } from '../../lib/i18n.js';
  import { Button, IconButton, Switch, Tag, Icon } from '../kit';

  /**
   * routine: { id, title, gist, prompt, enabled, quiet, bot: { name, orb }, schedule, next, nextTitle, nextIcon, last, lastTitle, lastBad, paused: { id, label } | null, errorPaused: { label } | null }
   * (строки приходят готовыми из app.js: дата и расписание считает он). Действия: onrun(id), onquiet(id, value), ontoggle(id, value), ondelete(id),
   * onresume(runId) для паузы по лимиту, onunpause(id) для паузы из-за серии ошибок.
   */
  let { routine, onrun, onresume, onunpause, onquiet, ontoggle, ondelete } = $props();
  // Задание короче трёх строк раскрывать нечего: кнопку показываем только для длинного.
  const text = $derived(routine.prompt || routine.gist || '');
  const long = $derived(text.length > 200 || text.split('\n').length > 3);
  let open = $state(false);
</script>

<article class="rc" class:is-off={!routine.enabled} data-routine={routine.id}>
  <header class="rc-head">
    <b class="rc-title" title={routine.title}>{routine.title}</b>
    <span class="rc-acts">
      <IconButton
        class="k-bell rc-quiet"
        name="bell"
        size="sm"
        label={tr('Писать в чат, только если есть что сказать')}
        title={`${routine.quiet ? tr('только важное') : tr('писать всё')}: ${tr('Писать в чат, только если есть что сказать')}`}
        aria-pressed={Boolean(routine.quiet)}
        onclick={() => onquiet?.(routine.id, !routine.quiet)}
      />
      <IconButton class="rc-run" name="play" size="sm" label={tr('Запустить')} onclick={() => onrun?.(routine.id)} />
      <Switch checked={routine.enabled} aria-label={tr('Включён')} onchange={(v) => ontoggle?.(routine.id, v)} />
      <IconButton class="rc-del" name="close" size="sm" label={tr('Удалить')} onclick={() => ondelete?.(routine.id)} />
    </span>
  </header>
  {#if text}
    <p class="rc-prompt" class:is-clamped={long && !open} class:is-open={long && open}>{text}</p>
    {#if long}
      <Button class="rc-more" size="sm" variant="ghost" aria-expanded={open} onclick={() => (open = !open)}>
        {open ? tr('Свернуть') : tr('Показать полностью')}
      </Button>
    {/if}
  {/if}
  <div class="rc-r2">
    <Tag title={tr('Бот')}>
      {#if routine.bot?.orb}<img src={routine.bot.orb} alt="" />{:else}<Icon name="user" size={13} />{/if}
      {routine.bot?.name || tr('бот удалён')}
    </Tag>
    {#if routine.schedule}<Tag title={tr('Когда')}><Icon name="repeat" size={13} />{routine.schedule}</Tag>{/if}
    {#if routine.next}<Tag tone="accent" title={routine.nextTitle || routine.next}><Icon name={routine.nextIcon || 'clock'} size={13} />{routine.next}</Tag>{/if}
    {#if routine.last}<Tag tone={routine.lastBad ? 'danger' : 'neutral'} title={routine.lastTitle || routine.last}><Icon name={routine.lastBad ? 'alert-circle' : 'check-long'} size={13} />{routine.last}</Tag>{/if}
    {#if routine.errorPaused}
      <Tag tone="danger" title={routine.errorPaused.label}><Icon name="alert-circle" size={13} />{routine.errorPaused.label}</Tag>
      <Button class="rc-unpause" size="sm" variant="ghost" onclick={() => onunpause?.(routine.id)}>{tr('Возобновить')}</Button>
    {/if}
    {#if routine.paused}
      <Tag tone="accent" title={routine.paused.label}><Icon name="clock" size={13} />{routine.paused.label}</Tag>
      <Button class="rc-resume" size="sm" variant="ghost" onclick={() => onresume?.(routine.paused.id)}>{tr('Продолжить сейчас')}</Button>
    {/if}
  </div>
</article>

<style>
  .rc {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: var(--k-space-3);
    min-width: 0;
    padding: var(--k-space-3) var(--k-space-4);
    border-radius: var(--k-radius-lg);
    background: var(--k-canvas);
    box-shadow: 0 0 0 1px var(--k-line);
    transition:
      box-shadow var(--k-dur-ui) var(--k-ease-out),
      opacity var(--k-dur-ui) var(--k-ease-out);
    animation: rc-in var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .rc:hover {
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--k-brand) 40%, var(--k-line));
  }
  .rc.is-off {
    opacity: 0.6;
  }
  .rc-head {
    display: flex;
    align-items: flex-start;
    gap: var(--k-space-3);
    min-width: 0;
  }
  .rc-title {
    flex: 1;
    min-width: 0;
    padding-top: var(--k-space-1);
    overflow-wrap: anywhere;
    font: 600 var(--k-text-md) / 1.3 var(--k-font-body);
    color: var(--k-text);
  }
  .rc-acts {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--k-space-0);
  }
  .rc-prompt {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 400 var(--k-text-sm) / 1.55 var(--k-font-body);
    color: var(--k-text-2);
  }
  /* Превью — сплошным текстом: пустые строки инструкции не съедают три строки предпросмотра. */
  .rc-prompt.is-clamped {
    white-space: normal;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
    -webkit-mask-image: linear-gradient(to bottom, currentColor 55%, transparent);
    mask-image: linear-gradient(to bottom, currentColor 55%, transparent);
  }
  .rc-prompt.is-open {
    max-height: 50vh;
    overflow-y: auto;
  }
  .rc :global(.rc-more) {
    align-self: flex-start;
  }
  .rc-r2 {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  @keyframes rc-in {
    from {
      opacity: 0;
      transform: translateY(6px);
      filter: blur(4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rc {
      animation: none;
    }
  }
</style>
