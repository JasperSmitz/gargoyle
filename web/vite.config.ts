import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';
import { loadEnv, type Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

/**
 * Dev-only backend stand-in (see dev/mock-api.ts): `VITE_MOCK=1 pnpm dev` serves the contract
 * examples instead of proxying to a backend; `VITE_MOCK=offline` makes every call fail.
 */
function mockApi(offline: boolean): Plugin {
  return {
    name: 'gargoyle-mock-api',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use('/api/v1', (req, res, next) => {
        // Node's request type is not visible without @types/node, which only this needs.
        const url = (req as { url?: string }).url ?? '/';
        const [path = '/', search = ''] = url.split('?');
        server
          .ssrLoadModule('/dev/mock-api.ts')
          .then((mod) => {
            const { status, body } = (mod as typeof import('./dev/mock-api')).respond(
              path,
              new URLSearchParams(search),
              offline,
            );
            res.statusCode = status;
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(body));
          })
          .catch(next);
      });
    },
  };
}

export default defineConfig(({ mode }) => {
  const mock = loadEnv(mode, '.', 'VITE_').VITE_MOCK;
  const mocked = mock === '1' || mock === 'offline';

  return {
    plugins: [svelte(), svelteTesting(), ...(mocked ? [mockApi(mock === 'offline')] : [])],
    server: {
      // `make dev`: the backend runs in mock mode on its default listen address.
      proxy: mocked ? undefined : { '/api': 'http://127.0.0.1:9077' },
    },
    build: {
      // Embedded by the binary (rust-embed). Vite's default `assets/[name]-[hash].ext` names are
      // relied upon: the backend serves everything except index.html as immutable.
      outDir: 'dist',
    },
    test: {
      include: ['src/**/*.test.ts', 'dev/**/*.test.ts'],
      // Component tests opt into jsdom with a `// @vitest-environment jsdom` comment.
      environment: 'node',
    },
  };
});
