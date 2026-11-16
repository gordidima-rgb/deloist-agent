<script>
  // Значки рабочих процессов: контурные, 24x24, толщина 2 — как остальные значки боковой панели.
  let { name, size = 16 } = $props();

  const PATHS = {
    trigger: ['M13 2 4 14h7l-1 8 9-12h-7z'],
    prompt: ['M4 6h16M4 12h16M4 18h10'],
    bot: ['M12 3v3', 'M7 8h10a3 3 0 0 1 3 3v5a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3v-5a3 3 0 0 1 3-3z', 'M9 13v1M15 13v1'],
    llm: ['M12 3l1.8 4.7L18.5 9.5 13.8 11.3 12 16l-1.8-4.7L5.5 9.5l4.7-1.8z', 'M18.5 15.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z'],
    condition: ['M12 3l8 9-8 9-8-9z'],
    approval: ['M12 3 4.5 6v5.5c0 4.2 3 7.6 7.5 9.5 4.5-1.9 7.5-5.3 7.5-9.5V6z', 'm8.8 12 2.4 2.4 4.2-4.6'],
    output: ['M4 12h12', 'm12 6 6 6-6 6', 'M20 4v16'],
    note: ['M5 4h14v10l-5 6H5z', 'M14 20v-6h5'],
    text: ['M5 6V4h14v2', 'M12 4v16', 'M9 20h6'],
    play: ['M7 4.5v15l12-7.5z'],
    stop: ['M6 6h12v12H6z'],
    plus: ['M12 5v14M5 12h14'],
    minus: ['M5 12h14'],
    fit: ['M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5'],
    map: ['M9 4 3 6.5v13.5l6-2.5 6 2.5 6-2.5V4l-6 2.5z', 'M9 4v13.5M15 6.5V20'],
    select: ['m5 3 14 7-6 2-2 6z'],
    hand: ['M8 12V6.5a1.5 1.5 0 0 1 3 0V11', 'M11 10V4.5a1.5 1.5 0 0 1 3 0V11', 'M14 10V6a1.5 1.5 0 0 1 3 0v9a6 6 0 0 1-6 6h-1a6 6 0 0 1-4.5-2L3 14.5a1.6 1.6 0 0 1 2.4-2L8 15'],
    search: ['M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z', 'm20 20-4-4'],
    undo: ['M9 14 4 9l5-5', 'M4 9h9a7 7 0 0 1 0 14h-2'],
    redo: ['m15 14 5-5-5-5', 'M20 9h-9a7 7 0 0 0 0 14h2'],
    graph: ['M6 4h4v4H6zM14 16h4v4h-4zM14 4h4v4h-4z', 'M10 6h4M8 8v5a3 3 0 0 0 3 3h3'],
    check: ['m5 12.5 4.5 4.5L19 7.5'],
    x: ['M6 6l12 12M18 6 6 18'],
    pause: ['M8 5v14M16 5v14'],
    skip: ['M5 12h14', 'm14 7 5 5-5 5'],
    chevron: ['m7 10 5 5 5-5'],
    copy: ['M9 9h10v11H9z', 'M5 15V4h10'],
    retry: ['M20 11a8 8 0 1 0-2.3 5.7', 'M20 4v7h-7'],
    clock: ['M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z', 'M12 8v4.5l3 1.5'],
    bell: ['M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z', 'M10 20a2 2 0 0 0 4 0'],
    trash: ['M5 7h14M10 4h4M7 7l1 13h8l1-13', 'M10 11v6M14 11v6'],
    dup: ['M8 8h12v12H8z', 'M4 16V4h12'],
    pencil: ['M4 20l1-4L16 5l3 3L8 19z', 'M14 7l3 3'],
    lock: ['M6 11h12v9H6z', 'M8.5 11V8a3.5 3.5 0 0 1 7 0v3'],
    more: ['M6 12h.01M12 12h.01M18 12h.01'],
    list: ['M4 6h16M4 12h16M4 18h16'],
    marker: ['M5 19l3-1 8-8-2-2-8 8z', 'M14 8l2-2 2 2-2 2', 'M4 21h8'],
    eraser: ['M7 20h12', 'M5 15l8-9 6 6-6 8H8z', 'M10 10l6 6'],
    shape: ['M4 4h8v8H4z', 'M16 21a4 4 0 1 0 0-8 4 4 0 0 0 0 8z'],
    history: ['M4 12a8 8 0 1 0 2.5-5.8', 'M4 4v5h5', 'M12 8v4.5l3 1.5'],
    versions: ['M12 3l9 5-9 5-9-5z', 'M3 13l9 5 9-5'],
    front: ['M8 8h12v12H8z', 'M4 16V4h12'],
    back: ['M4 4h12v12H4z', 'M8 20h12V8'],
    scissors: ['M6 7a2.5 2.5 0 1 0 0 .01', 'M6 17a2.5 2.5 0 1 0 0 .01', 'M8 8.5 20 18M8 15.5 20 6'],
    paste: ['M9 4h6v3H9z', 'M7 5H5v15h14V5h-2'],
    select_all: ['M4 4h16v16H4z', 'M8 8h8v8H8z'],
    unlock: ['M6 11h12v9H6z', 'M8.5 11V8a3.5 3.5 0 0 1 6.8-1'],
    run_step: ['M7 4.5v15l12-7.5z', 'M3 4v16'],
    link: ['M4 12h5l3-6 3 12 3-6h2'],
    soon: ['M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z', 'M12 8v4l2.5 1.5'],
  };

  const shapes = $derived(PATHS[name] || []);
  const dot = $derived(name === 'more');
</script>

<svg
  class="wf-ico"
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width={dot ? 3 : 2}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each shapes as d (d)}
    <path {d} />
  {/each}
</svg>
