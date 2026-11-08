// Bundle budget (M6.5): fails when the gzipped JavaScript in dist/ exceeds the limit. The UI is
// embedded in the binary and loaded on every dashboard visit, so growth should be a decision.
// Run after `pnpm build`: `pnpm size`.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const LIMIT_KB = 150;
const dist = fileURLToPath(new URL('../dist/', import.meta.url));

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

let all;
try {
  all = files(dist);
} catch {
  console.error('dist/ not found: run `pnpm build` first.');
  process.exit(1);
}

const scripts = all.filter((p) => p.endsWith('.js'));
if (scripts.length === 0) {
  // A failed or partial build must not pass as "0 KiB, within budget".
  console.error('No .js files in dist/: run `pnpm build` first.');
  process.exit(1);
}

const kb = (bytes) => (bytes / 1024).toFixed(1);
let total = 0;
for (const path of scripts) {
  const content = readFileSync(path);
  const gzipped = gzipSync(content, { level: 9 }).length;
  total += gzipped;
  console.log(`${relative(dist, path)}  ${kb(content.length)} KiB  (${kb(gzipped)} KiB gzipped)`);
}

const verdict = total <= LIMIT_KB * 1024 ? 'within' : 'OVER';
console.log(`Total JS: ${kb(total)} KiB gzipped, ${verdict} the ${LIMIT_KB} KiB budget.`);
if (total > LIMIT_KB * 1024) process.exit(1);
