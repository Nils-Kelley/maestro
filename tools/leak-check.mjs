#!/usr/bin/env node
// Keep private names out of the repository. Scans files for home folders, user names in paths, email addresses, and the names
// of your own projects, and fails if it finds any.
//
//   node tools/leak-check.mjs             what is staged for the next commit (the pre-commit hook)
//   node tools/leak-check.mjs --all       every tracked file and every new file git would add
//   node tools/leak-check.mjs --history   every commit: its message, its author's email, and the lines it added
//   node tools/leak-check.mjs --install   make it this clone's pre-commit and pre-push hook
//
// Your private words never enter the repository. They come from two git-ignored files in the repo folder:
//   maestro.config.json   every project's name and folder name (4 letters or more) counts as private
//   .private-words        one regular expression per line (# starts a comment; "!name" lets a project name through)
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync, chmodSync } from 'node:fs';
import { dirname, join, basename } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const git = (...args) => execFileSync('git', ['-C', root, ...args], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const mode = process.argv[2] || '--staged';

if (mode === '--install') {
  const hooks = git('rev-parse', '--git-path', 'hooks').trim();
  for (const [name, arg] of [['pre-commit', ''], ['pre-push', ' --all']]) {
    const file = join(root, hooks, name);
    if (existsSync(file) && !readFileSync(file, 'utf8').includes('leak-check')) { console.error(`${name}: a hook is already there; add "node tools/leak-check.mjs${arg}" to it yourself`); continue; }
    writeFileSync(file, `#!/bin/sh\n# written by tools/leak-check.mjs --install\nexec node "$(git rev-parse --show-toplevel)/tools/leak-check.mjs"${arg}\n`);
    try { chmodSync(file, 0o755); } catch (_) {}
    console.log(`installed ${name}`);
  }
  process.exit(0);
}

// ---- what counts as a leak ----
const OK_USERS = new Set(['me', 'you', 'user', 'username', 'runner', 'public', 'default', 'someone', 'name']);
const OK_MAIL = /(@users\.noreply\.github\.com|@example\.(com|org|net)|^git@|^noreply@)/i;
const rules = [
  { why: 'a home folder', re: /\/home\/([A-Za-z_][\w.-]*)/g, skip: (m) => OK_USERS.has(m[1].toLowerCase()) },
  { why: 'a Windows user folder', re: /[A-Za-z]:\\{1,2}Users\\{1,2}([^\\\/\s"'<>]+)/g, skip: (m) => OK_USERS.has(m[1].toLowerCase()) },
  { why: 'an email address', re: /[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}/g, skip: (m) => OK_MAIL.test(m[0]) },
];
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const words = existsSync(join(root, '.private-words')) ? readFileSync(join(root, '.private-words'), 'utf8').split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith('#')) : [];
const allowed = new Set(words.filter((w) => w.startsWith('!')).map((w) => w.slice(1).toLowerCase()));
for (const w of words.filter((w) => !w.startsWith('!'))) {
  try { rules.push({ why: 'a private word (.private-words)', re: new RegExp(w, 'gi') }); } catch (e) { console.error(`.private-words: "${w}" is not a regular expression (${e.message})`); process.exit(2); }
}
let fromConfig = 0;
if (existsSync(join(root, 'maestro.config.json'))) {
  try {
    const names = new Set();
    for (const p of JSON.parse(readFileSync(join(root, 'maestro.config.json'), 'utf8')).projects || []) {
      for (const n of [p.name, String(p.root || '').split(/[\\/]/).pop()]) if (n) names.add(String(n).toLowerCase());
    }
    names.delete(basename(root).toLowerCase()); names.delete('maestro');
    const list = [...names].filter((n) => n.length >= 4 && !allowed.has(n)).sort((a, b) => b.length - a.length);
    fromConfig = list.length;
    if (list.length) rules.push({ why: 'one of your project names (maestro.config.json)', re: new RegExp(`(?<![A-Za-z0-9_-])(${list.map(esc).join('|')})(?![A-Za-z0-9_-])`, 'gi') });
  } catch (e) { console.error(`maestro.config.json could not be read: ${e.message}`); }
}

// ---- what is scanned ----
const SKIP = /(^|\/)(Cargo\.lock|ui\/vendor\/.*|.*\.(png|ico|jpg|jpeg|gif|woff2?|exe|dll))$/;
const found = [];
const scan = (where, text) => {
  text.split('\n').forEach((line, i) => {
    for (const r of rules) {
      r.re.lastIndex = 0; let m;
      while ((m = r.re.exec(line))) {
        if (!(r.skip && r.skip(m))) found.push(`${where}:${i + 1}  ${r.why}: ${line.slice(Math.max(0, m.index - 30), m.index + m[0].length + 30).trim()}`);
        if (m[0] === '') r.re.lastIndex++;
      }
    }
  });
};
if (mode === '--history') {
  for (const sha of git('rev-list', '--all').split('\n').filter(Boolean)) {
    const short = sha.slice(0, 8);
    const [an, ae, cn, ce] = git('show', '-s', '--format=%an%x1f%ae%x1f%cn%x1f%ce', sha).trim().split('\x1f');
    for (const [who, mail] of [[an, ae], [cn, ce]]) if (!OK_MAIL.test(mail)) found.push(`commit ${short}  the author's email is in the commit (${who}): use GitHub's noreply address if you would rather not publish it`);
    scan(`commit ${short} message`, git('show', '-s', '--format=%B', sha));
    let file = '';
    const added = [];
    for (const line of git('show', '--format=', '--unified=0', '--no-color', sha).split('\n')) {
      if (line.startsWith('+++ b/')) file = line.slice(6);
      else if (line.startsWith('+') && !line.startsWith('+++') && !SKIP.test(file)) added.push([file, line.slice(1)]);
    }
    for (const [f, l] of added) scan(`commit ${short} ${f}`, l);
  }
} else {
  const names = mode === '--all' ? git('ls-files', '--cached', '--others', '--exclude-standard') : git('diff', '--cached', '--name-only', '--diff-filter=ACMR');
  for (const f of [...new Set(names.split('\n').filter(Boolean))]) {
    if (SKIP.test(f)) continue;
    let text;
    try { text = mode === '--all' ? readFileSync(join(root, f), 'utf8') : git('show', `:${f}`); } catch (_) { continue; } // deleted, or a folder
    if (text.includes('\u0000')) continue;
    scan(f, text);
    scan(`${f} (its name)`, f);
  }
}
if (found.length) {
  console.error(`leak check: ${found.length} place${found.length === 1 ? '' : 's'} to look at\n` + [...new Set(found)].join('\n'));
  console.error('\nUse an invented name instead (the preview project is "tidepool"). A project name that is an ordinary word can be let through with "!name" in .private-words.');
  process.exit(1);
}
console.log(`leak check: clean (${rules.length} patterns${fromConfig ? `, ${fromConfig} project names from your config` : ''}${words.length ? `, ${words.length} lines of .private-words` : ''})`);
