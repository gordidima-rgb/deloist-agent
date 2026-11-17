<script>
  // Line diff of the person's text against the catalog's new one (LCS, texts are short).
  import { tr } from '../../lib/i18n.js';

  let { mine = '', theirs = '' } = $props();

  const rows = $derived.by(() => {
    const a = String(mine).split('\n');
    const b = String(theirs).split('\n');
    if (a.length * b.length > 250000) return [...a.map((text) => ({ kind: 'del', text })), ...b.map((text) => ({ kind: 'add', text }))];
    const n = a.length;
    const m = b.length;
    const lcs = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    const out = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
      if (a[i] === b[j]) {
        out.push({ kind: 'same', text: a[i] });
        i++;
        j++;
      } else if (lcs[i + 1][j] >= lcs[i][j + 1]) out.push({ kind: 'del', text: a[i++] });
      else out.push({ kind: 'add', text: b[j++] });
    }
    while (i < n) out.push({ kind: 'del', text: a[i++] });
    while (j < m) out.push({ kind: 'add', text: b[j++] });
    return out;
  });
</script>

<div class="diff" role="table" aria-label={tr('Разница между вашей и новой версией')}>
  {#each rows as row, index (index)}
    <div class="cat-row {row.kind}" role="row">
      <span class="sign" aria-hidden="true">{row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ' '}</span>
      <span class="line" role="cell">{row.text || ' '}</span>
    </div>
  {/each}
</div>
<p class="legend"><span class="del">− {tr('ваша версия')}</span> <span class="add">+ {tr('новая версия')}</span></p>

<style>
  .diff {
    max-height: 280px;
    overflow: auto;
    border-radius: var(--k-radius-sm, 12px);
    background: var(--k-surface-2);
    font: 400 var(--k-text-xs, 12px) / 1.5 var(--k-font-mono, monospace);
  }
  .cat-row {
    display: flex;
    gap: 6px;
    padding: 0 var(--k-space-2, 8px);
  }
  .line {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .cat-row.add {
    background: var(--k-brand-tint);
  }
  .cat-row.del {
    background: var(--k-attention-tint);
    color: var(--k-attention-text);
  }
  .legend {
    display: flex;
    gap: var(--k-space-3, 12px);
    margin: 4px 0 0;
    font-size: var(--k-text-xs, 12px);
    color: var(--k-text-3, inherit);
  }
  .legend .del {
    color: var(--k-attention-text);
  }
  .legend .add {
    color: var(--k-brand-text);
  }
</style>
