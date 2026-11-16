import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Тесты интерфейса: jsdom, Svelte в браузерном режиме (иначе mount() недоступен).
export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ['browser'] },
  test: { environment: 'jsdom', include: ['src/**/*.test.js'] },
});
