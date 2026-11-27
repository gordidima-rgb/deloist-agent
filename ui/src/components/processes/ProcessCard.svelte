<script>
  // Краткая карточка процесса «Углей»: сверху название и действия, под ним задание на всю ширину (3 строки с затуханием,
  // «Показать полностью» раскрывает), ниже метки: бот, когда, следующий запуск (оранжевым), прошлый запуск, пауза.
  // «Показать цепочку» раскрывает шаги списком; править цепочку — на холсте («Открыть на холсте»).
  // Выключенный процесс бледнеет, а не исчезает.
  import { tr, trServer } from '../../lib/i18n.js';
  import { api as realApi } from '../../lib/api.js';
  import { orderSteps } from '../../lib/processSteps.js';
  import { toMs } from '../../lib/workflows/graph.js';
  import { agoText, kindTitle, nextRunText, scheduleText, statusLabel, targetLabel, triggerLabel, whenText } from '../workflows/Labels.svelte';
  import WfIcon from '../workflows/WfIcon.svelte';
  import { Button, IconButton, Switch, Tag, Icon } from '../kit';

  /**
   * item — строка списка `GET /api/workflows` (id, title, enabled, last_run, failures, stop_reason, error_paused, updated_at, card);
   * bots [{id,name}]. wait — запуск на паузе из-за лимита (`GET /api/needs`: id, until, reason) или пусто.
   * Действия: onrun(id), onquiet(id, value), ontoggle(id, value), onresume(id), onresumenow(runId), ondelete(id), onopen(id) — холст.
   */
  let { item, bots = [], api = realApi, onrun, onquiet, ontoggle, onresume, wait = null, onresumenow, ondelete, onopen } = $props();

  const card = $derived(item.card ?? {});
  const text = $derived(card.task || '');
  const long = $derived(text.length > 200 || text.split('\n').length > 3);
  const hasBot = $derived(card.bot_id != null);
  const botName = (id) => bots.find((b) => b.id === id)?.name || (id ? tr('бот удалён') : tr('Бот не выбран'));
  const when = $derived(card.trigger === 'schedule' ? scheduleText(card.schedule) : triggerLabel(card.trigger));
  const next = $derived(item.enabled && !item.error_paused && card.next_run > 0 ? nextRunText(card.next_run) : '');
  const last = $derived(item.last_run);
  const lastBad = $derived(last && ['failed', 'stopped'].includes(last.status));
  const reasons = { network: tr('сеть'), login: tr('нужен вход'), check: tr('проверка не прошла'), stuck: tr('застрял') };
  const why = $derived(reasons[item.stop_reason] || '');
  const pauseText = $derived(tr('на паузе: серия ошибок ({n}) — {why}', { n: item.failures || 3, why: why || tr('ошибка') }));

  const waitReasons = { limit: tr('лимит'), budget: tr('предел токенов'), event: tr('ждёт события'), network: tr('сеть') };
  const waitText = $derived(wait ? tr('на паузе до {time} ({why})', { time: nextRunText(wait.until), why: waitReasons[wait.reason] || '' }) : '');

  let open = $state(false);
  let confirming = $state(false);
  let chain = $state(false);
  let steps = $state.raw(null);
  let stepsError = $state('');

  // Удаляет второе нажатие; не нажали за 3 секунды — всё как было.
  function askDelete() {
    confirming = true;
    setTimeout(() => (confirming = false), 3200);
  }

  // Цепочка читается по запросу и заново, если процесс правили после прошлого раскрытия.
  $effect(() => {
    const stamp = item.updated_at;
    if (!chain) return;
    stepsError = '';
    api(`/api/workflows/${encodeURIComponent(item.id)}`)
      .then((res) => {
        if (stamp === item.updated_at) steps = orderSteps(res.workflow?.graph);
      })
      .catch((err) => {
        stepsError = trServer(err?.message || tr('Не получилось'));
      });
  });

  const clip = (value, n = 90) => {
    const line = String(value || '').replace(/\s+/g, ' ').trim();
    return line.length > n ? `${line.slice(0, n)}…` : line;
  };

  // Что шаг делает, одной строкой: расписание у триггера, бот и задание у бота, место у вывода, иначе первый текст шага.
  function brief(node) {
    const f = node.fields || {};
    if (node.kind === 'trigger') return node.schedule ? scheduleText(node.schedule) : triggerLabel(f.trigger);
    if (node.kind === 'bot') return [botName(f.bot_id), clip(f.task, 70)].filter(Boolean).join(': ');
    if (node.kind === 'output') return targetLabel(f.target);
    const first = Object.values(f).find((v) => typeof v === 'string' && v.trim());
    return clip(first);
  }
</script>

<article class="pc" class:is-off={!item.enabled} data-process={item.id}>
  <header class="pc-head">
    <b class="pc-title" title={item.title}>{item.title}</b>
    <span class="pc-acts">
      {#if hasBot}
        <IconButton
          class="k-bell pc-quiet"
          name="bell"
          size="sm"
          label={tr('Писать в чат, только если есть что сказать')}
          title={`${card.quiet ? tr('только важное') : tr('писать всё')}: ${tr('Писать в чат, только если есть что сказать')}`}
          aria-pressed={Boolean(card.quiet)}
          onclick={() => onquiet?.(item.id, !card.quiet)}
        />
      {/if}
      <IconButton class="pc-run" name="play" size="sm" label={tr('Запустить')} onclick={() => onrun?.(item.id)} />
      <Switch checked={item.enabled} aria-label={tr('Включён')} onchange={(v) => ontoggle?.(item.id, v)} />
      {#if confirming}
        <Button class="pc-del" variant="danger-solid" size="sm" title={tr('Нажмите ещё раз, чтобы удалить')} aria-label={tr('Удалить')} onclick={() => ondelete?.(item.id)}>{tr('Удалить?')}</Button>
      {:else}
        <IconButton class="pc-del" name="close" size="sm" label={tr('Удалить')} onclick={askDelete} />
      {/if}
    </span>
  </header>
  {#if text}
    <p class="pc-prompt" class:is-clamped={long && !open} class:is-open={long && open}>{text}</p>
    {#if long}
      <Button class="pc-more" size="sm" variant="ghost" aria-expanded={open} onclick={() => (open = !open)}>
        {open ? tr('Свернуть') : tr('Показать полностью')}
      </Button>
    {/if}
  {/if}
  <div class="pc-tags">
    {#if hasBot}
      <Tag title={tr('Бот')}><Icon name="user" size={13} />{botName(card.bot_id)}</Tag>
    {/if}
    {#if when}<Tag title={tr('Когда')}><Icon name="repeat" size={13} />{when}</Tag>{/if}
    {#if next}<Tag tone="accent" title={tr('Следующий запуск: {time}', { time: next })}><Icon name="clock" size={13} />{next}</Tag>{/if}
    {#if last}
      <Tag tone={lastBad ? 'danger' : 'neutral'} title={`${tr('Последний запуск {time}', { time: whenText(toMs(last.at)) })}: ${statusLabel(last.status)}${why && lastBad ? ` (${why})` : ''}`}>
        <Icon name={lastBad ? 'alert-circle' : 'check-long'} size={13} />{agoText(last.at)}
      </Tag>
    {/if}
    {#if wait}
      <Tag tone="accent" title={waitText}><Icon name="clock" size={13} />{waitText}</Tag>
      <Button class="pc-resume-now" size="sm" variant="ghost" onclick={() => onresumenow?.(wait.id)}>{tr('Продолжить сейчас')}</Button>
    {/if}
    {#if item.error_paused}
      <Tag tone="danger" title={pauseText}><Icon name="alert-circle" size={13} />{pauseText}</Tag>
      <Button class="pc-resume" size="sm" variant="ghost" onclick={() => onresume?.(item.id)}>{tr('Продолжить')}</Button>
    {/if}
  </div>
  <div class="pc-foot">
    <Button class="pc-chain-btn" size="sm" variant="ghost" iconName={chain ? 'chevron-down' : 'chevron-right'} aria-expanded={chain} onclick={() => (chain = !chain)}>
      {chain ? tr('Скрыть цепочку') : tr('Показать цепочку')}
    </Button>
  </div>
  {#if chain}
    <div class="pc-chain">
      {#if stepsError}
        <p class="pc-note">{stepsError}</p>
      {:else if !steps}
        <p class="pc-note">{tr('Загрузка…')}</p>
      {:else}
        <ol class="pc-steps">
          {#each steps as node (node.id)}
            <li class="pc-step">
              <span class="pc-step-ico"><WfIcon name={node.kind} size={15} /></span>
              <b>{kindTitle(node.kind)}</b>
              <span class="pc-step-brief">{brief(node)}</span>
            </li>
          {/each}
        </ol>
      {/if}
      <Button class="pc-canvas" size="sm" variant="secondary" iconName="workflow" onclick={() => onopen?.(item.id)}>{tr('Открыть на холсте')}</Button>
    </div>
  {/if}
</article>

<style>
  .pc {
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
    animation: pc-in var(--k-dur-appear) var(--k-ease-spring) both;
  }
  .pc:hover {
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--k-brand) 40%, var(--k-line));
  }
  .pc.is-off {
    opacity: 0.6;
  }
  .pc-head {
    display: flex;
    align-items: flex-start;
    gap: var(--k-space-3);
    min-width: 0;
  }
  .pc-title {
    flex: 1;
    min-width: 0;
    padding-top: var(--k-space-1);
    overflow-wrap: anywhere;
    font: 600 var(--k-text-md) / 1.3 var(--k-font-body);
    color: var(--k-text);
  }
  .pc-acts {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--k-space-0);
  }
  .pc-prompt {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 400 var(--k-text-sm) / 1.55 var(--k-font-body);
    color: var(--k-text-2);
  }
  /* Превью — сплошным текстом: пустые строки инструкции не съедают три строки предпросмотра. */
  .pc-prompt.is-clamped {
    white-space: normal;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
    -webkit-mask-image: linear-gradient(to bottom, currentColor 55%, transparent);
    mask-image: linear-gradient(to bottom, currentColor 55%, transparent);
  }
  .pc-prompt.is-open {
    max-height: 50vh;
    overflow-y: auto;
  }
  .pc :global(.pc-more),
  .pc :global(.pc-chain-btn) {
    align-self: flex-start;
  }
  .pc-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pc-foot {
    display: flex;
  }
  .pc-chain {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--k-space-3);
    padding-top: var(--k-space-3);
    border-top: 1px solid var(--k-line);
  }
  .pc-note {
    margin: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    color: var(--k-text-3);
  }
  .pc-steps {
    display: flex;
    flex-direction: column;
    gap: var(--k-space-2);
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .pc-step {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--k-space-1) var(--k-space-2);
    min-width: 0;
    font: 400 var(--k-text-sm) / 1.45 var(--k-font-body);
    color: var(--k-text);
  }
  .pc-step-ico {
    align-self: center;
    display: inline-flex;
    color: var(--k-brand);
  }
  .pc-step b {
    font-weight: 600;
  }
  .pc-step-brief {
    flex: 1 1 12ch;
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--k-text-2);
  }
  @keyframes pc-in {
    from {
      opacity: 0;
      transform: translateY(6px);
      filter: blur(4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pc {
      animation: none;
    }
  }
</style>
