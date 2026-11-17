<script>
import { Checkbox } from './kit';
  import { tr } from '../lib/i18n.js';
  import { api as apiFetch } from '../lib/api.js';

  let {
    api = apiFetch,
    onclose = null,
    onimported = null,
  } = $props();

  let step = $state('sources');
  let sources = $state([]);
  let selected = $state(new Set());
  let topics = $state([]);
  let redactions = $state(0);
  let facts = $state([]);
  let agents = $state([]);
  let model = $state('');
  let isLocal = $state(false);
  let loading = $state(false);
  let error = $state('');
  let hermes = $state({ found: false, root: '', profiles: [] });
  let hermesChosen = $state(new Set());
  let hermesBusy = $state(false);
  let hermesResult = $state('');
  let hermesFailed = $state(false);

  async function loadSources() {
    loading = true; error = '';
    try { sources = await api('/api/know-me/sources'); }
    catch (e) { error = e?.message || String(e); }
    loading = false;
  }

  async function loadHermes() {
    try {
      const res = await api('/api/import/hermes');
      const profiles = Array.isArray(res?.profiles) ? res.profiles : [];
      hermes = { found: res?.found === true && profiles.length > 0, root: res?.root || '', profiles };
      hermesChosen = new Set(profiles.map((p) => p.name));
    } catch {
      hermes = { found: false, root: '', profiles: [] };
      hermesChosen = new Set();
    }
  }

  $effect(() => {
    loadSources();
    loadHermes();
  });

  function toggleHermes(name) {
    const next = new Set(hermesChosen);
    if (next.has(name)) next.delete(name); else next.add(name);
    hermesChosen = next;
  }

  async function importHermes() {
    if (!hermesChosen.size || hermesBusy) return;
    hermesBusy = true; hermesResult = ''; hermesFailed = false;
    try {
      const res = await api('/api/import/hermes', { method: 'POST', body: { profiles: [...hermesChosen] } });
      const ids = Array.isArray(res?.botIds) ? res.botIds : [];
      hermesResult = tr('Перенесено ботов: {n}', { n: ids.length });
      onimported?.(ids);
    } catch (e) {
      hermesResult = e?.message || String(e);
      hermesFailed = true;
    }
    hermesBusy = false;
  }

  function toggle(id) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id); else next.add(id);
    selected = next;
  }

  async function read() {
    loading = true; error = '';
    try {
      const res = await api('/api/know-me/read', { method: 'POST', body: { sources: [...selected] } });
      topics = res.topics || [];
      redactions = res.redactions || 0;
      // Кому уйдут темы, человек видит ДО отправки.
      model = res.model || '';
      isLocal = res.local === true;
      step = 'topics';
    } catch (e) { error = e?.message || String(e); }
    loading = false;
  }

  async function propose() {
    loading = true; error = '';
    try {
      const res = await api('/api/know-me/propose', { method: 'POST', body: { topics } });
      facts = (res.facts || []).map((f) => (typeof f === 'string' ? f : (f?.title || f?.fact || '')));
      agents = res.agents || [];
      step = 'profile';
    } catch (e) { error = e?.message || String(e); }
    loading = false;
  }

  async function save() {
    loading = true; error = '';
    try {
      await api('/api/know-me/save', { method: 'POST', body: { facts, agents } });
      step = 'saved';
    } catch (e) { error = e?.message || String(e); }
    loading = false;
  }

  function close() { onclose?.(); }
  function removeAgent(i) { agents = agents.filter((_, idx) => idx !== i); }
  function updateFact(i, v) { facts = facts.map((f, idx) => idx === i ? v : f); }
  function removeFact(i) { facts = facts.filter((_, idx) => idx !== i); }
</script>

<div class="km-wrap">
  <div class="km-card">
    <h2 class="km-title">{tr('Познакомиться быстрее')}</h2>

    {#if error}
      <p class="km-error">{error}</p>
    {/if}

    {#if step === 'sources'}
      {#if hermes.found}
        <section class="km-hermes">
          <h3 class="km-hermes-title">{tr('Hermes Agent')}</h3>
          <ul class="km-hermes-list">
            {#each hermes.profiles as p (p.name)}
              <li>
                <Checkbox
                  checked={hermesChosen.has(p.name)}
                  onchange={() => toggleHermes(p.name)}
                  label={`${p.name} — ${tr('характер {n} знаков, {m} записей памяти', { n: p.personaChars, m: p.memoryEntries })}`}
                />
              </li>
            {/each}
          </ul>
          <div class="km-hermes-actions">
            <button class="km-btn" disabled={hermesBusy || hermesChosen.size === 0} onclick={importHermes}>{tr('Перенести как ботов')}</button>
          </div>
          {#if hermesResult}
            <p class="km-note" class:km-error={hermesFailed}>{hermesResult}</p>
          {/if}
        </section>
      {/if}
      <p class="km-note">{tr('Отметь источники и нажми «Прочитать». Ничего не читается, пока не отметишь.')}</p>
      {#if loading && !sources.length}
        <p class="km-note">{tr('Ищу историю разговоров на этом компьютере…')}</p>
      {/if}
      <ul class="km-sources">
        {#each sources as s}
          <li>
            <label>
              <input type="checkbox" checked={selected.has(s.id)} disabled={!s.found} onchange={() => toggle(s.id)} />
              <span>{s.name}</span>
              <span class="km-count">{s.found ? s.count : tr('скоро')}</span>
            </label>
          </li>
        {/each}
      </ul>
      <div class="km-actions">
        <button class="km-btn" disabled={loading || selected.size === 0} onclick={read}>{tr('Прочитать')}</button>
        <button class="km-btn ghost" onclick={close}>{tr('Отмена')}</button>
      </div>

    {:else if step === 'topics'}
      <p class="km-note">{tr('Найдено редактур: {n}', { n: redactions })}</p>
      <ul class="km-topics">
        {#each topics as t}
          <li>{t}</li>
        {/each}
      </ul>
      <p class="km-note">{tr('Темы отправятся модели {model}', { model })}{#if !isLocal} {tr('(облачной модели)')}{/if}</p>
      <div class="km-actions">
        <button class="km-btn" disabled={loading} onclick={propose}>{tr('Отправить модели {model}', { model })}</button>
        <button class="km-btn ghost" onclick={() => step = 'sources'}>{tr('Назад')}</button>
      </div>

    {:else if step === 'profile'}
      <div class="km-facts">
        {#each facts as f, i}
          <input class="km-input" value={f} oninput={(e) => updateFact(i, e.currentTarget.value)} />
          <button class="km-x" onclick={() => removeFact(i)}>×</button>
        {/each}
      </div>
      <div class="km-agents">
        {#each agents as a, i}
          <div class="km-agent">
            <strong>{a.name || a?.name}</strong> — {a.role || a?.role}
            <button class="km-x" onclick={() => removeAgent(i)}>×</button>
          </div>
        {/each}
      </div>
      <div class="km-actions">
        <button class="km-btn" disabled={loading} onclick={save}>{tr('Сохранить')}</button>
        <button class="km-btn ghost" onclick={cancel}>{tr('Отмена')}</button>
      </div>

    {:else if step === 'saved'}
      <p class="km-note">{tr('Сохранено.')}</p>
      <div class="km-actions">
        <button class="km-btn" onclick={close}>{tr('Закрыть')}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .km-wrap { position: fixed; inset: 0; z-index: 10000; display: grid; place-items: center; background: rgba(0,0,0,.45); padding: var(--k-space-4, 16px); }
  .km-card { width: min(560px, 100%); max-height: 80vh; overflow: auto; border-radius: var(--k-radius-lg, 26px); background: var(--k-surface, #fff); color: var(--k-text, #121212); box-shadow: var(--k-shadow-3, 0 28px 70px -12px rgba(0,0,0,.22)); padding: var(--k-space-5, 20px); display: flex; flex-direction: column; gap: var(--k-space-3, 12px); }
  .km-title { margin: 0; font: 800 var(--k-text-xl, 22px)/var(--k-leading-tight,1.2) var(--k-font-display, system-ui); }
  .km-note { margin: 0; color: var(--k-text-3, #6e6e6e); font: 400 var(--k-text-sm,13px)/1.4 var(--k-font-body,system-ui); }
  .km-error { color: #c00; margin: 0; }
  .km-hermes { display: flex; flex-direction: column; gap: var(--k-space-2,8px); padding: var(--k-space-3,12px); border: 1px solid var(--k-line,#e8e8e8); border-radius: var(--k-radius-md,18px); }
  .km-hermes-title { margin: 0; font: 700 var(--k-text-md,15px)/var(--k-leading-tight,1.2) var(--k-font-body,system-ui); }
  .km-hermes-list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: var(--k-space-1,4px); }
  /* Скрытый input флажка абсолютный: без опоры он уезжает за край окна. */
  .km-hermes-list :global(.k-check) { position: relative; }
  .km-hermes-actions { display: flex; justify-content: flex-end; }
  .km-sources, .km-topics { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: var(--k-space-2,8px); }
  .km-sources li label { display: flex; align-items: center; gap: 8px; }
  .km-count { margin-left: auto; color: var(--k-text-3,#6e6e6e); font-size: 12px; }
  .km-actions { display: flex; gap: 8px; justify-content: flex-end; }
  .km-btn { height: 36px; padding: 0 var(--k-space-4,16px); border: 0; border-radius: var(--k-radius-pill,999px); background: var(--k-brand,#025bff); color: var(--k-on-brand,#fff); font: 600 var(--k-text-sm,13px)/1 var(--k-font-body,system-ui); cursor: pointer; }
  .km-btn.ghost { background: transparent; color: var(--k-text,#121212); border: 1px solid var(--k-border,#ddd); }
  .km-btn:disabled { opacity: .5; cursor: default; }
  .km-input { width: 100%; padding: 6px 8px; border: 1px solid var(--k-border,#ddd); border-radius: 8px; }
  .km-x { border: 0; background: transparent; cursor: pointer; font-size: 18px; }
  .km-agent { display: flex; align-items: center; gap: 8px; }
</style>
