import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Вторая IIFE-библиотека: раздел «Рабочие процессы» (см. src/workflows-entry.js).
// Собирается после основной (та очищает web/ui), поэтому emptyOutDir выключен.
export default defineConfig({
  plugins: [svelte()],
  define: { 'process.env.NODE_ENV': '"production"' },
  build: {
    outDir: '../web/ui',
    emptyOutDir: false,
    target: 'es2020',
    minify: true,
    sourcemap: false,
    cssCodeSplit: false,
    lib: {
      entry: 'src/workflows-entry.js',
      name: 'DeloistWorkflowsBundle',
      formats: ['iife'],
      fileName: () => 'workflows.js',
      cssFileName: 'workflows',
    },
  },
});
