<script>
  // Шарик бота с другого ПК: тот же стеклянный шар, что у своих ботов (CrewAvatar), цвет и номер шара проходят проверку.
  // Без движка шаров (публичная сборка, нет WebGL2) остаётся кружок с буквой.
  import { safeColor } from '../../lib/remote.js';

  let { bot, busy = false, size = 40 } = $props();
  let host = $state();
  let live = $state(false);
  const mark = $derived(String(bot?.emoji || '').trim() || Array.from(String(bot?.name || '?'))[0]?.toUpperCase() || '?');
  const neutral = () => (typeof document !== 'undefined' && getComputedStyle(document.documentElement).getPropertyValue('--k-pix-wait-lo').trim()) || 'gray';
  const color = $derived(safeColor(bot?.color) || neutral());
  const skin = $derived(Number.isInteger(bot?.skin) && bot.skin >= 0 && bot.skin < 1000 ? bot.skin : 0);

  $effect(() => {
    if (!host || typeof window === 'undefined' || !window.CrewAvatar?.mount) return;
    host.textContent = '';
    let avatar = null;
    try {
      avatar = window.CrewAvatar.mount(host, { size, color, skin, name: String(bot?.name || '') });
    } catch {
      avatar = null;
    }
    live = Boolean(avatar);
    return () => {
      avatar?.destroy?.();
      live = false;
    };
  });
</script>

<span class="orb" class:is-busy={busy} class:is-live={live} style:width="{size}px" style:height="{size}px" style:font-size="{Math.round(size * 0.42)}px" style:background={live ? undefined : color} aria-hidden="true"><span bind:this={host} class="orb-gl"></span>{#if !live}{mark}{/if}</span>

<style>
  .orb {
    position: relative; flex: none; display: grid; place-items: center; border-radius: 50%; color: var(--k-on-brand); font-weight: 600; overflow: visible;
    transition: box-shadow 0.15s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .orb.is-busy { box-shadow: 0 0 0 2px var(--k-surface), 0 0 0 4px var(--k-brand); }
  .orb.is-live { background: none; }
  .orb-gl { position: absolute; inset: 0; }
</style>
