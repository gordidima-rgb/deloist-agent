<script>
  // Окно модели у шапки чата: один клик по плашке модели открывает выбор прямо здесь, для этого бота.
  // Сверху тот же ModelPicker (открытый сразу), под списком строки «Усилие» и настройки агента
  // (например, «Скорость») компактными селектами кита и ссылка на все настройки модели.
  import { tr } from '../lib/i18n.js';
  import ModelPicker from './ModelPicker.svelte';
  import Select from './kit/Select.svelte';

  let {
    providers = [],
    plan = null,
    loading = false,
    settings = {},
    value = { driver: '', model: '' },
    effort = '',
    agentOptions = {},
    defaultLabel = '',
    anchor = null,
    onselect,
    oneffort,
    onoption,
    onconnect,
    onmore,
    onclose,
  } = $props();

  // Какая модель на самом деле отвечает: своя у бота или из общих настроек.
  const driver = $derived(value.driver || settings.driver || '');
  const provider = $derived(providers.find((p) => p.driver === driver) || null);
  const FALLBACK = [
    { id: 'low', name: tr('Низкое') },
    { id: 'medium', name: tr('Среднее') },
    { id: 'high', name: tr('Высокое') },
  ];
  // Список усилий приходит от провайдера (`efforts`). Нет списка: у API-драйверов low/medium/high,
  // у агентов ACP, программ и демо усилия нет.
  const efforts = $derived.by(() => {
    if (provider && Array.isArray(provider.efforts)) return provider.efforts;
    if (!driver || driver === 'demo' || driver === 'cli' || driver === 'acp' || driver.startsWith('acp:')) return [];
    return FALLBACK;
  });
  const nameIn = (list, id) => list.find((e) => e.id === id)?.name || '';
  const effortOptions = $derived([
    { value: '', label: tr('Авто') + (nameIn(efforts, provider?.effortDefault) ? ` (${nameIn(efforts, provider.effortDefault)})` : '') },
    ...efforts.map((e) => ({ value: e.id, label: e.name || e.id })),
  ]);
  const effortValue = $derived(efforts.some((e) => e.id === effort) ? effort : '');
  const extra = $derived(Array.isArray(provider?.options) ? provider.options.filter((o) => Array.isArray(o.values) && o.values.length) : []);
  const optionChoices = (o) => [
    { value: '', label: nameIn(o.values, o.default) || tr('по умолчанию') },
    ...o.values.filter((v) => v.id !== o.default).map((v) => ({ value: v.id, label: v.name || v.id })),
  ];
</script>

{#snippet footer()}
  {#if efforts.length}
    <div class="cmp-row">
      <label class="cmp-label" for="cmp-effort">{tr('Усилие')}</label>
      <Select id="cmp-effort" class="cmp-select" options={effortOptions} value={effortValue} onchange={(e) => oneffort?.(e.currentTarget.value)} />
    </div>
  {/if}
  {#each extra as o (o.id)}
    <div class="cmp-row">
      <label class="cmp-label" for="cmp-opt-{o.id}">{o.name || o.id}</label>
      <Select id="cmp-opt-{o.id}" class="cmp-select" options={optionChoices(o)} value={o.values.some((v) => v.id === agentOptions?.[o.id]) ? agentOptions[o.id] : ''} onchange={(e) => onoption?.(o.id, e.currentTarget.value)} />
    </div>
  {/each}
  <button type="button" class="cmp-more" onclick={() => onmore?.()}>{tr('Все настройки модели…')}</button>
{/snippet}

<ModelPicker {providers} {plan} {loading} {settings} {value} {defaultLabel} activeDriver={settings.driver || ''} {anchor} {onselect} {onconnect} {onclose} startOpen bare {footer} />

<style>
  .cmp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--k-space-3);
    margin-bottom: var(--k-space-2);
  }
  .cmp-label {
    font-size: var(--k-text-sm);
    color: var(--k-text-2);
  }
  .cmp-row :global(.k-select) {
    min-width: 150px;
  }
  .cmp-more {
    margin-top: var(--k-space-1);
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    font-size: var(--k-text-xs);
    color: var(--k-text-2);
    text-decoration: underline;
    cursor: pointer;
  }
  .cmp-more:hover {
    color: var(--k-text);
  }
  .cmp-more:focus-visible {
    outline: none;
    box-shadow: var(--k-focus);
  }
</style>
