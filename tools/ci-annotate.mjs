// Turn the compiler's errors in a build log into annotations on the workflow run, so that what went wrong on a system nobody
// has at hand (a macOS or Linux runner) can be read from the run's page and its public record.   node tools/ci-annotate.mjs <log>
import { readFileSync } from 'node:fs';

const lines = readFileSync(process.argv[2], 'utf8').replace(/\r/g, '').split('\n');
let n = 0;
for (let i = 0; i < lines.length && n < 40; i++) {
  const m = lines[i].match(/^(error|warning)(\[[A-Z]\d+\])?: (.+)$/);
  if (!m || /^(aborting due to|could not compile|\d+ warnings? emitted)/.test(m[3]) || /generated \d+ warning/.test(m[3])) continue;
  // the place follows on a line like "   --> src/main.rs:12:34"
  let file = '', line = '';
  for (let k = i + 1; k < Math.min(i + 4, lines.length); k++) { const at = lines[k].match(/^\s*--> (.+?):(\d+):\d+/); if (at) { file = at[1].replace(/\\/g, '/'); line = at[2]; break; } }
  // what the compiler adds under it ("expected …, found …", "help: …") is the useful part
  const more = []; for (let k = i + 1; k < Math.min(i + 14, lines.length) && !/^(error|warning)/.test(lines[k]); k++) { const t = lines[k].replace(/^\s*\d*\s*\|?\s?/, '').trim(); if (/^(= |help|note|\^|expected|found)/.test(t) || /\^+/.test(lines[k])) more.push(t); }
  const text = `${m[3]}${more.length ? ' | ' + more.slice(0, 5).join(' | ') : ''}`.replace(/%/g, '%25').replace(/\n/g, '%0A').slice(0, 900);
  const where = file ? ` file=src-tauri/${file.replace(/^src-tauri\//, '')},line=${line}` : '';
  console.log(`::${m[1] === 'error' ? 'error' : 'warning'}${where}::${text}`);
  n++;
}
// a failed test: its name and what it said
for (let i = 0; i < lines.length && n < 60; i++) {
  const t = lines[i].match(/^---- (\S+) stdout ----$/); if (!t) continue;
  const said = []; for (let k = i + 1; k < Math.min(i + 12, lines.length) && !/^----/.test(lines[k]) && lines[k].trim() !== 'failures:'; k++) if (lines[k].trim()) said.push(lines[k].trim());
  console.log(`::error::test ${t[1]} failed | ${said.join(' | ').replace(/%/g, '%25').slice(0, 900)}`); n++;
}
console.log(`${n} annotation${n === 1 ? '' : 's'}`);
