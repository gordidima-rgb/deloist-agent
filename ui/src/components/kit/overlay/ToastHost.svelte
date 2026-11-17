<script>
  // Единственный хост уведомлений. Очередь и таймеры живут в ui/src/lib/toast.js,
  // хост только показывает снимок и отдаёт нажатие действия.
  import Toast from './Toast.svelte';

  let { subscribe, dismiss } = $props();

  let items = $state([]);

  $effect(() =>
    subscribe((next) => {
      items = next;
    }),
  );
</script>

<div class="k-toast-host" aria-live="polite" aria-atomic="false">
  {#each items as item (item.id)}
    <Toast
      text={item.message}
      action={item.action
        ? {
            label: item.action.label,
            onclick: () => {
              try {
                item.action.onclick?.();
              } finally {
                dismiss(item.id);
              }
            },
          }
        : null}
    />
  {/each}
</div>
