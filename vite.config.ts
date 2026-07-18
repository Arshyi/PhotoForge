import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    conditions: ['browser']
  },
  clearScreen: false,
  server: {
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**']
    }
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['src/test-setup.ts'],
    include: ['src/**/*.test.ts']
  }
});
