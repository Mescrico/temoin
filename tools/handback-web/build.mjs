// Builds server/web/handback.js (loaded by the admin page on demand).
import { build } from 'esbuild';
import { readFileSync } from 'node:fs';

const deps = Object.keys(JSON.parse(readFileSync('package.json', 'utf8')).dependencies);
const notices = deps.map((d) => {
  const p = JSON.parse(readFileSync(`node_modules/${d}/package.json`, 'utf8'));
  return ` *   ${p.name} ${p.version} (${p.license})`;
});
await build({
  entryPoints: ['src/handback.js'],
  bundle: true,
  format: 'iife',
  globalName: 'TemoinHandback',
  minify: true,
  target: 'es2020',
  legalComments: 'inline',
  outfile: '../../server/web/handback.js',
  banner: {
    js: `/*! Témoin hand-back for the admin page (AGPL-3.0-or-later), built from tools/handback-web.\n * Bundled libraries:\n${notices.join('\n')}\n */`,
  },
});
console.log('server/web/handback.js built');
