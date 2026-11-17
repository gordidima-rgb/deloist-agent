<script>
  import { tick } from 'svelte';
  import { tr } from '../lib/i18n.js';
  import { detectMentionTrigger, TRIGGER_RE } from '../lib/mention.js';

  let { bots = [], input = null } = $props();

  let open = $state(false);
  let query = $state('');
  let selected = $state(0);
  let pos = $state({ left: 0, top: 0, width: 240 });

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [...bots];
    const prefix = [];
    const contains = [];
    for (const b of bots) {
      const name = (b.name || '').toLowerCase();
      if (name.startsWith(q)) prefix.push(b);
      else if (name.includes(q)) contains.push(b);
    }
    return [...prefix, ...contains];
  });

  function place() {
    if (!input) return;
    const r = input.getBoundingClientRect();
    pos = { left: r.left, top: r.top, width: Math.min(r.width, 320) };
  }

  function insert(bot) {
    if (!bot || !input) return;
    const caret = input.selectionStart ?? input.value.length;
    const before = input.value.slice(0, caret);
    const m = before.match(TRIGGER_RE);
    if (!m) { open = false; return; }
    const triggerStart = m.index + m[1].length;
    const inserted = m[2] + (bot.name || '') + ' ';
    const newValue = input.value.slice(0, triggerStart) + inserted + input.value.slice(caret);
    input.value = newValue;
    const newCaret = triggerStart + inserted.length;
    input.setSelectionRange(newCaret, newCaret);
    open = false;
    query = '';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  $effect(() => {
    if (!input) return;
    const onInput = () => {
      const caret = input.selectionStart ?? input.value.length;
      const before = input.value.slice(0, caret);
      const trig = detectMentionTrigger(before);
      if (trig) {
        query = trig.query;
        selected = 0;
        place();
        open = true;
      } else {
        open = false;
      }
    };
    const onKeydown = (e) => {
      if (!open) return;
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        selected = Math.min(selected + 1, Math.max(filtered.length - 1, 0));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        selected = Math.max(selected - 1, 0);
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        if (filtered.length) {
          e.preventDefault();
          insert(filtered[selected]);
        }
      } else if (e.key === 'Escape') {
        e.preventDefault();
        open = false;
      }
    };
    input.addEventListener('input', onInput);
    // Захват: список забирает Enter/Tab раньше, чем поле отправит сообщение (там проверяют defaultPrevented).
    input.addEventListener('keydown', onKeydown, true);
    return () => {
      input.removeEventListener('input', onInput);
      input.removeEventListener('keydown', onKeydown, true);
    };
  });
</script>

{#if open}
  <div class="k-menu k-sharpen-in mn-pop" style:left="{pos.left}px" style:top="{pos.top}px" style:width="{pos.width}px" role="listbox" aria-label={tr('Агенты')}>
    <div class="k-menu__note mn-group">{tr('Агенты')} <span class="k-menu__value">{bots.length}</span></div>
    {#if filtered.length}
      <ul class="mn-list">
        {#each filtered as bot, i (bot.id)}
          <li>
            <button type="button" class="k-menu__item mn-item" class:mn-sel={i === selected} role="option" aria-selected={i === selected} onclick={() => insert(bot)} onmouseenter={() => (selected = i)}>
              <span class="mn-avatar" aria-hidden="true">{bot.emoji || (bot.name || '?').charAt(0)}</span>
              <span class="mn-name">{bot.name}</span>
              {#if bot.role}<span class="k-menu__value mn-role">{bot.role}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="k-menu__note mn-empty">{tr('Ничего не нашлось')}</p>
    {/if}
  </div>
{/if}

<style>
  .mn-pop {
    position: fixed;
    z-index: 1000;
    transform: translateY(-100%);
    margin-top: calc(var(--k-space-2) * -1);
    max-height: 260px;
    overflow: auto;
  }
  .mn-group {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .mn-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .mn-item.mn-sel {
    background: var(--k-ink-2);
  }
  .mn-avatar {
    flex: none;
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--k-radius-sm);
    background: var(--k-ink-3);
    color: var(--k-on-ink);
    font: 700 var(--k-text-xs) / 1 var(--k-font-body);
  }
  .mn-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mn-role {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
