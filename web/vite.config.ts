import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [svelte()],
  server: {
    // `make dev`: the backend runs in mock mode on its default listen address.
    proxy: { '/api': 'http://127.0.0.1:9077' },
  },
  build: {
    // Embedded by the binary (rust-embed). Vite's default `assets/[name]-[hash].ext` names are
    // relied upon: the backend serves everything except index.html as immutable.
    outDir: 'dist',
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
});
