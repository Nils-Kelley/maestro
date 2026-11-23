// Each page's script must parse: one that does not leaves the app on its intro graphic with nothing running, and cargo cannot see it.
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const dir = mkdtempSync(join(tmpdir(), 'maestro-'));
for (const name of ['index.html', 'remote.html']) {
  const page = readFileSync(join(here, '..', 'ui', name), 'utf8');
  const m = page.match(/<script>([\s\S]*)<\/script>/);
  if (!m) { console.error(`ui/${name} has no inline script`); process.exit(1); }
  const file = join(dir, name.replace('.html', '.js'));
  writeFileSync(file, m[1]);
  execFileSync(process.execPath, ['--check', file], { stdio: 'inherit' });
}
console.log('the page scripts parse');
