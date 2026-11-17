<script>
  // Знак работы. В ките «крутится» только девять пикселей (docs/design/kit.md, правило 5),
  // поэтому спиннер — это .k-pix--act, а не бесконечное вращение кольца.
  import { tr } from '../../../lib/i18n.js';

  let { label = '', size = 'md', class: className = '', ...rest } = $props();

  const name = $derived(label || tr('Загружаю…'));
</script>

<span class="k-spinner {className}" class:k-spinner--sm={size === 'sm'} class:k-spinner--lg={size === 'lg'} role="status" aria-label={name} {...rest}>
  <span class="k-pix k-pix--act" class:k-pix--lg={size === 'lg'} aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>
</span>
