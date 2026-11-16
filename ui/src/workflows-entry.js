// Отдельная сборка раздела «Рабочие процессы» (web/ui/workflows.js + workflows.css).
// Холст на Svelte Flow тяжёлый, поэтому web/app.js подгружает файл при первом открытии раздела,
// а основной kit.js остаётся лёгким. Раздел монтируется через window.DeloistWorkflows.mount.
import { mountComponent } from './lib/mount.svelte.js';
import WorkflowsScreen from './components/workflows/WorkflowsScreen.svelte';

window.DeloistWorkflows = {
  mount: (target, props) => mountComponent(WorkflowsScreen, target, props),
};
