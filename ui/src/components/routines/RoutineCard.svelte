<script>
  // Карточка рутины «Углей»: в одной строке название, описание с обрезкой и действия (колокольчик, запуск, переключатель, удаление);
  // ниже метки: бот, как часто, следующий запуск (оранжевым), прошлый запуск. Выключенная рутина бледнеет, а не исчезает.
  import { tr } from '../../lib/i18n.js';
  import { Button, IconButton, Switch, Tag, Icon } from '../kit';

  /**
   * routine: { id, title, gist, enabled, quiet, bot: { name, orb }, schedule, next, nextTitle, nextIcon, last, lastTitle, lastBad, paused: { id, label } | null }
   * (строки приходят готовыми из app.js: дата и расписание считает он). Действия: onrun(id), onquiet(id, value), ontoggle(id, value), ondelete(id), onresume(runId) для паузы по лимиту.
   */
  let { routine, onrun, onresume, onquiet, ontoggle, ondelete } = $props();
</script>

<article class="rc" class:is-off={!routine.enabled} data-routine={routine.id}>
  <div class="rc-r1">
    <b class="rc-title" title={routine.title}>{routine.title}</b>
    {#if routine.gist}<span class="rc-gist" title={routine.gist}>{routine.gist}</span>{/if}
    <span class="rc-acts">
      <IconButton
        class="k-bell rt-quiet"
        name="bell"
        size="sm"
        label={tr('Писать в чат, только если есть что сказать')}
        title={`${routine.quiet ? tr('только важное') : tr('писать всё')}: ${tr('Писать в чат, только если есть что сказать')}`}
        aria-pressed={Boolean(routine.quiet)}
        onclick={() => onquiet?.(routine.id, !routine.quiet)}
      />
      <IconButton class="rt-run" name="play" size="sm" label={tr('Запустить')} onclick={() => onrun?.(routine.id)} />
      <Switch checked={routine.enabled} aria-label={tr('Включён')} onchange={(v) => ontoggle?.(routine.id, v)} />
      <IconButton class="rt-del" name="close" size="sm" label={tr('Удалить')} onclick={() => ondelete?.(routine.id)} />
    </span>
  </div>
  <div class="rc-r2">
    <Tag title={tr('Бот')}>
      {#if routine.bot?.orb}<img src={routine.bot.orb} alt="" />{:else}<Icon name="user" size={13} />{/if}
      {routine.bot?.name || tr('бот удалён')}
    </Tag>
    {#if routine.schedule}<Tag title={tr('Когда')}><Icon name="repeat" size={13} />{routine.schedule}</Tag>{/if}
    {#if routine.next}<Tag tone="accent" title={routine.nextTitle || routine.next}><Icon name={routine.nextIcon || 'clock'} size={13} />{routine.next}</Tag>{/if}
    {#if routine.last}<Tag tone={routine.lastBad ? 'danger' : 'neutral'} title={routine.lastTitle || routine.last}><Icon name={routine.lastBad ? 'alert-circle' : 'check-long'} size={13} />{routine.last}</Tag>{/if}
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
    gap: var(--k-space-3);
    padding: var(--k-space-3) var(--k-space-3) var(--k-space-3) var(--k-space-4);
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
  .rc-r1 {
    display: flex;
    align-items: center;
    gap: var(--k-space-3);
    min-width: 0;
  }
  .rc-title {
    flex: none;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 600 var(--k-text-md) / 1.3 var(--k-font-body);
    color: var(--k-text);
  }
  .rc-gist {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 400 var(--k-text-sm) / 1.3 var(--k-font-body);
    color: var(--k-text-3);
  }
  .rc-acts {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--k-space-0);
    margin-left: auto;
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
