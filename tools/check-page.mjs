// The page's script must parse: one that does not leaves the app on its intro graphic with nothing running, and cargo cannot see it.
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const page = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'ui', 'index.html'), 'utf8');
const m = page.match(/<script>([\s\S]*)<\/script>/);
if (!m) { console.error('ui/index.html has no inline script'); process.exit(1); }
const file = join(mkdtempSync(join(tmpdir(), 'maestro-')), 'page.js');
writeFileSync(file, m[1]);
execFileSync(process.execPath, ['--check', file], { stdio: 'inherit' });
console.log('the page script parses');
