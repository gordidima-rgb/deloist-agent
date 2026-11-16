import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Одна IIFE-библиотека: старые страницы грузят её обычным <script>, ES-модули им не нужны.
// Результат лежит в ../web/ui и коммитится (cargo build не требует Node).
export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: '../web/ui',
    emptyOutDir: true,
    target: 'es2020',
    minify: true,
    sourcemap: false,
    cssCodeSplit: false,
    lib: {
      entry: 'src/index.js',
      name: 'DeloistUIBundle',
      formats: ['iife'],
      fileName: () => 'kit.js',
      cssFileName: 'kit',
    },
  },
});
