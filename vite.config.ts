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
    // Keep Vite's file traversal inside this checkout. Besides being a safer
    // desktop default, this avoids probing unrelated parent directories when
    // the app is run from a locked-down Windows profile.
    fs: {
      allow: [process.cwd()]
    },
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
