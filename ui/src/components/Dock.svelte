<script>
  // Док ботов «Тушь и сигнал» (docs/design/kit.md): чёрный островок у края, кольцо лимита вокруг аватара
  // каждого бота, спрятанный лоток с микрофоном, шестерёнкой и «+», меню дока и голосовой поток.
  // Окно, область мыши, перетаскивание, чат и карточки ведёт web/dock.js; аватары он же монтирует в
  // ячейки, которые компонент отдаёт через onface. Стили — классы .k-* из /kit.css и dock.html.
  // Бесконечных анимаций нет: всё движение — короткие транзиции в ответ на событие.
  import { tr } from '../lib/i18n.js';
  import MicButton from './MicButton.svelte';

  import Icon from './kit/Icon.svelte';
  let {
    bots = [],
    side = 'right',
    voice = { state: 'idle', level: 0, progress: 0, botId: '', error: '' },
    voiceAvailable = false,
    voiceTargetId = '',
    settings = { autohide: false, layer: 'top', voiceTarget: '' },
    speaking = { botId: '', level: 0 },
    hiddenBots = [],
    onseat,
    onstopspeak,
    onshowbot,
    onseatenter,
    onseatleave,
    onface,
    onfaceremove,
    onadd,
    onmic,
    onsetting,
    onhide,
    onvoiceaction,
    ongrip,
    onregion,
  } = $props();

  let root = $state();
  let grip = $state();
  let gear = $state();
  let mic = $state();
  let menuOpen = $state(false);
  let voiceListOpen = $state(false);
  let menuBottom = $state(0);
  let menuMax = $state(400);
  let hitId = $state('');
  let fly = $state(null); // { x, y, dx, dy }
  let prevVoice = 'idle';

  const left = $derived(side === 'left');
  const voiceOn = $derived(voice.state && voice.state !== 'idle');
  const showBadge = $derived(['downloading', 'loading', 'recording', 'transcribing'].includes(voice.state));
  const pct = (x) => Math.round(Math.min(1, Math.max(0, x)) * 100);
  // Цвет бота, к которому уйдёт голос: вид B микрофона красится в него (kit.css, «Mic looks»).
  const targetColor = $derived(bots.find((bot) => bot.id === (voice.botId || voiceTargetId))?.color || '#ffffff');
  let showListOpen = $state(false);

  // Нажатие на говорящего бота останавливает чтение вслух, а не открывает чат.
  function pressSeat(bot, el) {
    if (speaking.botId === bot.id) onstopspeak?.(bot.id);
    else onseat?.(bot.id, el);
  }

  function seatLabel(bot) {
    if (!bot.limit) return bot.name;
    return tr('{name}, осталось {percent} %', { name: bot.name, percent: pct(bot.limit.left) });
  }

  // Ячейка лица: в неё dock.js монтирует аватар бота (слизь, стекло, аниме или экран).
  // Одна функция на бота: новая функция на каждой отрисовке заново монтировала бы аватар.
  const attachers = new Map();
  const faceOf = (id) => {
    if (!attachers.has(id)) {
      attachers.set(id, (el) => {
        onface?.(id, el);
        return () => {
          attachers.delete(id);
          onfaceremove?.(id, el);
        };
      });
    }
    return attachers.get(id);
  };

  $effect(() => {
    if (grip) ongrip?.(grip);
  });

  // Полёт точки к боту и ответ его кольца: один раз на отправку.
  $effect(() => {
    const state = voice.state;
    const was = prevVoice;
    prevVoice = state;
    if (state === 'sending' && was !== 'sending' && root && mic) {
      const seat = root.querySelector(`[data-seat="${CSS.escape(voice.botId || voiceTargetId || '')}"] .k-ring`);
      if (!seat) return;
      const r0 = root.getBoundingClientRect();
      const m = mic.getBoundingClientRect();
      const s = seat.getBoundingClientRect();
      fly = { x: m.left - r0.left + m.width / 2, y: m.top - r0.top + m.height / 2, dx: s.left - m.left + (s.width - m.width) / 2, dy: s.top - m.top + (s.height - m.height) / 2, id: voice.botId || voiceTargetId };
    }
  });

  function flight(node) {
    const f = fly;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const bend = left ? 26 : -26;
    const anim = node.animate(
      [
        { transform: 'translate(0, 0) scale(1)' },
        { transform: `translate(${f.dx * 0.5 + bend}px, ${f.dy * 0.5}px) scale(1.15)`, offset: 0.5 },
        { transform: `translate(${f.dx}px, ${f.dy}px) scale(0.6)` },
      ],
      { duration: reduce ? 1 : 460, easing: 'cubic-bezier(0.32, 0.72, 0, 1)' },
    );
    anim.onfinish = () => {
      fly = null;
      hitId = f.id;
      setTimeout(() => (hitId = ''), 400);
    };
    return () => anim.cancel();
  }

  function openMenu() {
    if (menuOpen) {
      closeMenu();
      return;
    }
    const r0 = root.getBoundingClientRect();
    const g = gear.getBoundingClientRect();
    menuBottom = r0.bottom - g.bottom;
    // Меню растёт вверх от шестерёнки: не выше края экрана, длинный список ботов прокручивается.
    menuMax = Math.max(160, g.bottom - 8);
    menuOpen = true;
    voiceListOpen = false;
    requestAnimationFrame(() => {
      root.querySelector('.dock-menu .k-menu__item')?.focus({ preventScroll: true });
      onregion?.();
    });
  }
  function closeMenu(refocus = false) {
    if (!menuOpen) return;
    menuOpen = false;
    voiceListOpen = false;
    if (refocus) gear?.focus({ preventScroll: true });
    requestAnimationFrame(() => onregion?.());
  }
  function set(patch) {
    onsetting?.(patch);
  }
  function onkey(event) {
    if (event.key === 'Escape' && menuOpen) {
      event.stopPropagation();
      closeMenu(true);
    }
  }
  function outside(event) {
    if (menuOpen && !event.target.closest?.('.dock-menu, .k-gear')) closeMenu();
  }

  const targetName = $derived(settings.voiceTarget ? (bots.find((b) => b.id === settings.voiceTarget)?.name ?? tr('Последнему')) : tr('Последнему'));
  const tip = $derived.by(() => {
    if (voice.state === 'downloading') return { text: tr('Скачиваю модель речи, 0,6 ГБ · {percent} %', { percent: pct(voice.progress || 0) }), action: tr('Отменить'), act: 'cancel' };
    if (voice.state === 'loading') return { text: tr('Загружаю модель в память') };
    if (voice.state === 'error') {
      if (voice.error === 'money') return { text: tr('Не хватает денег на распознавание'), action: tr('Пополнить'), act: 'topup' };
      if (voice.error === 'mic') return { text: tr('Микрофон недоступен'), action: tr('Настройки'), act: 'settings' };
      return { text: voice.message || tr('Голос не распознан') };
    }
    return null;
  });
  const tipTop = $derived.by(() => {
    if (!tip || !mic || !root) return 0;
    voice.state; // пересчитать при смене состояния
    const r0 = root.getBoundingClientRect();
    const m = mic.getBoundingClientRect();
    return m.top - r0.top + m.height / 2;
  });
  $effect(() => {
    tip;
    requestAnimationFrame(() => onregion?.());
  });
</script>

<svelte:window onkeydown={onkey} onpointerdown={outside} />

<div class="k-dock dock-v2" class:k-dock--left={left} class:is-voice={voiceOn} style:--k-voice-target={targetColor} bind:this={root}>
  <div class="k-edge" class:k-edge--left={left}>
    <i class="k-fillet k-fillet--top"></i>
    <div class="k-island dock-island">
      {#each bots as bot (bot.id)}
        <button
          type="button"
          class="k-seat"
          class:is-voice-target={showBadge && bot.id === voiceTargetId}
          class:is-voice-hit={hitId === bot.id}
          class:is-speaking={speaking.botId === bot.id}
          data-seat={bot.id}
          aria-label={speaking.botId === bot.id ? tr('{name} читает ответ вслух: нажми, чтобы остановить', { name: bot.name }) : seatLabel(bot)}
          onclick={(e) => pressSeat(bot, e.currentTarget)}
          onpointerenter={(e) => onseatenter?.(bot.id, e.currentTarget)}
          onpointerleave={() => onseatleave?.(bot.id)}
          onfocus={(e) => onseatenter?.(bot.id, e.currentTarget)}
          onblur={() => onseatleave?.(bot.id)}
        >
          <span
            class="k-ring"
            class:is-free={!bot.limit}
            class:is-low={bot.limit && bot.limit.left > 0 && bot.limit.left < 0.15}
            class:is-empty={bot.limit && bot.limit.left <= 0}
            style:--ring-color={bot.color}
            style:--left={bot.limit ? bot.limit.left : 0}
            style:--spk={speaking.botId === bot.id ? Math.min(1, Math.max(0, speaking.level || 0)) : undefined}
          >
            <svg viewBox="0 0 52 52" aria-hidden="true"
              ><circle class="k-ring__track" cx="26" cy="26" r="24.75" pathLength="100" /><circle class="k-ring__arc" cx="26" cy="26" r="24.75" pathLength="100" /></svg
            >
            <span class="k-ring__face" {@attach faceOf(bot.id)}></span>
          </span>
          {#if bot.queued || bot.failed}
            <span class="avatar-badges">
              {#if bot.queued}<span class="avatar-badge">{bot.queued}</span>{/if}
              {#if bot.failed}<span class="avatar-badge is-error" role="img" aria-label={tr('Последний ход не удался')}>!</span>{/if}
            </span>
          {/if}
          {#if bot.needs}
            <span class="seat-dot is-need" role="img" aria-label={tr('Ждёт тебя')}></span>
          {:else if bot.unread}
            <span class="seat-dot"></span>
          {/if}
          <span class="k-seat__voice" aria-hidden="true"
            ><Icon name="mic-round" size={null} stroke={3} /></span
          >
        </button>
      {/each}
    </div>
    <i class="k-fillet k-fillet--bottom"></i>
  </div>

  <div class="k-tray-clip">
    <div class="k-gear-tray">
      {#if voiceAvailable}
        <span class="mic-slot" bind:this={mic}>
          <MicButton state={voice.state || 'idle'} level={voice.level || 0} progress={voice.progress || 0} onpress={() => onmic?.()} />
        </span>
      {/if}
      <button type="button" class="k-gear" bind:this={gear} aria-haspopup="menu" aria-expanded={menuOpen} aria-label={tr('Настройки дока')} title={tr('Настройки дока')} onclick={openMenu}>
        <Icon name="gear" size={null} stroke={1.8} class="k-i" />
      </button>
      <button type="button" class="k-tray-add" aria-label={tr('Новый бот')} title={tr('Новый бот')} onclick={() => onadd?.()}>
        <Icon name="plus" size={null} stroke={1.8} class="k-i" />
      </button>
      <svg class="k-grip" bind:this={grip} viewBox="0 0 28 10" role="separator" aria-orientation="horizontal" aria-label={tr('Потяни вдоль края, чтобы сдвинуть, или от стены, чтобы перенести')}
        ><title>{tr('Потяни вдоль края, чтобы сдвинуть, или от стены, чтобы перенести')}</title><path d="M3 3c5.5 4.5 16.5 4.5 22 0" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" /></svg
      >
    </div>
  </div>

  <div class="dock-menu" class:open={menuOpen} class:left style:bottom="{menuBottom}px" role="menu" aria-label={tr('Настройки дока')} data-region={menuOpen ? '' : undefined}>
    <div class="k-menu" style:max-height="{menuMax}px">
      {#if menuOpen}
        <div class="k-sharpen-in">
          <div class="dock-menu__title">{tr('Док')}</div>
          <button type="button" class="k-menu__item" role="menuitemcheckbox" aria-checked={settings.autohide} onclick={() => set({ dockAutohide: !settings.autohide })}>
            <Icon name="dock-hide" size={null} stroke={1.8} class="k-i" />
            {tr('Прятать к краю')}<span class="k-switch"></span>
          </button>
          <button type="button" class="k-menu__item" role="menuitemcheckbox" aria-checked={settings.layer !== 'normal'} onclick={() => set({ dockLayer: settings.layer === 'normal' ? 'top' : 'normal' })}>
            <Icon name="layers" size={null} stroke={1.8} class="k-i" />
            {tr('Поверх всех окон')}<span class="k-switch"></span>
          </button>
          <button type="button" class="k-menu__item" role="menuitem" onclick={() => { closeMenu(); set({ dockEdge: left ? 'right' : 'left' }); }}>
            <Icon name="arrows-swap" size={null} stroke={1.8} class="k-i" />
            {tr('Край экрана')}<span class="k-menu__value">{left ? tr('Слева') : tr('Справа')}</span>
          </button>
          <button type="button" class="k-menu__item" role="menuitem" aria-haspopup="true" aria-expanded={voiceListOpen} onclick={() => (voiceListOpen = !voiceListOpen)}>
            <Icon name="mic" size={null} stroke={1.8} class="k-i" />
            {tr('Голос уходит')}<span class="k-menu__value">{targetName}</span>
          </button>
          {#if voiceListOpen}
            <div class="dock-menu__sub" role="group" aria-label={tr('Голос уходит')}>
              <button type="button" class="k-menu__item" role="menuitemradio" aria-checked={!settings.voiceTarget} onclick={() => set({ voiceTarget: 'last' })}>
                {tr('Последнему, с кем говорил')}{#if !settings.voiceTarget}<span class="k-menu__value">✓</span>{/if}
              </button>
              {#each bots as bot (bot.id)}
                <button type="button" class="k-menu__item" role="menuitemradio" aria-checked={settings.voiceTarget === bot.id} onclick={() => set({ voiceTarget: bot.id })}>
                  <span class="dock-menu__dot" style:background={bot.color}></span>{bot.name}{#if settings.voiceTarget === bot.id}<span class="k-menu__value">✓</span>{/if}
                </button>
              {/each}
            </div>
          {/if}
          {#if hiddenBots.length}
            <button type="button" class="k-menu__item" role="menuitem" aria-haspopup="true" aria-expanded={showListOpen} onclick={() => (showListOpen = !showListOpen)}>
              <Icon name="eye" size={null} stroke={1.8} class="k-i" />
              {tr('Показать в доке…')}<span class="k-menu__value">{hiddenBots.length}</span>
            </button>
            {#if showListOpen}
              <div class="dock-menu__sub" role="group" aria-label={tr('Показать в доке…')}>
                {#each hiddenBots as bot (bot.id)}
                  <button type="button" class="k-menu__item" role="menuitem" onclick={() => onshowbot?.(bot.id)}>
                    <span class="dock-menu__dot" style:background={bot.color}></span>{bot.name}
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
          <div class="k-menu__sep"></div>
          <button type="button" class="k-menu__item" role="menuitem" onclick={() => { closeMenu(); onhide?.(); }}>
            <Icon name="eye-off" size={null} stroke={1.8} class="k-i" />
            {tr('Скрыть док')}
          </button>
        </div>
      {/if}
    </div>
  </div>

  {#if tip}
    <div class="k-voice-tip dock-voice-tip" class:left style:top="{tipTop}px" data-region role="status">
      <span>{tip.text}</span>
      {#if tip.action}<button type="button" onclick={() => onvoiceaction?.(tip.act)}>{tip.action}</button>{/if}
    </div>
  {/if}

  {#if fly}
    <span class="k-voice-fly" style:left="{fly.x}px" style:top="{fly.y}px" {@attach flight}></span>
  {/if}
</div>

<style>
  .dock-v2 {
    position: relative;
  }
  .dock-island {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--k-dock-gap, 6px);
    padding: 12px 0;
    max-height: calc(100vh - 140px);
    overflow-y: auto;
    scrollbar-width: none;
  }
  .dock-island::-webkit-scrollbar {
    display: none;
  }
  .mic-slot {
    display: grid;
  }
  .dock-menu {
    position: absolute;
    right: calc(100% + 6px);
    z-index: 6;
    width: 264px;
    clip-path: inset(calc(100% - 40px) 0 0 calc(100% - 40px) round 20px);
    opacity: 0;
    pointer-events: none;
    transition:
      clip-path var(--k-dur-exit, 260ms) var(--k-ease-in, ease-in),
      opacity 0s linear var(--k-dur-exit, 260ms);
  }
  .dock-menu.left {
    right: auto;
    left: calc(100% + 6px);
    clip-path: inset(calc(100% - 40px) calc(100% - 40px) 0 0 round 20px);
  }
  .dock-menu.open {
    clip-path: inset(0 round 20px);
    opacity: 1;
    pointer-events: auto;
    transition:
      clip-path var(--k-dur-morph, 420ms) var(--k-ease-morph, ease-out),
      opacity 0s;
  }
  .dock-menu .k-menu {
    box-shadow: var(--k-ink-rim);
    overflow-y: auto;
    scrollbar-width: none;
  }
  .dock-menu__title {
    padding: 8px 12px 6px;
    font: 800 14px/1.3 var(--k-font-display);
    color: var(--k-on-ink);
  }
  .dock-menu__sub {
    margin: 2px 0 4px 18px;
    padding-left: 8px;
    border-left: 1px solid var(--k-ink-line);
    max-height: 240px;
    overflow-y: auto;
  }
  .dock-menu__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .dock-voice-tip {
    position: absolute;
    right: calc(100% + 8px);
    z-index: 6;
    transform: translateY(-50%);
  }
  .dock-voice-tip.left {
    right: auto;
    left: calc(100% + 8px);
  }
  @media (prefers-reduced-motion: reduce) {
    .dock-menu,
    .dock-menu.open {
      transition: none;
    }
  }
</style>
