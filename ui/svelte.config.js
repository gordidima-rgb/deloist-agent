import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
  // CSS компонентов уходит во внешний kit.css, а не в JS: страницы подключают его <link>.
  compilerOptions: { css: 'external', runes: true },
};
