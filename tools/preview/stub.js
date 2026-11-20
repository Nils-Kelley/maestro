// A stand-in for the Rust side of Maestro: every command the page invokes answers from the invented project below.
// Nothing here is read from disk. `window.__PREVIEW.emit(event, payload)` lets a test play a session's output.
(() => {
  const scene = new URLSearchParams(location.search).get('scene') || 'full';
  document.documentElement.style.background = '#0b0e13'; // the app's window is see-through; a browser tab needs a backdrop
  const day = (n) => { const d = new Date(); d.setDate(d.getDate() - n); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };
  const now = Math.floor(Date.now() / 1000);
  const handlers = {};
  const emit = (ev, payload) => (handlers[ev] || []).forEach((h) => h({ payload }));

  const rules = [
    { f: 'harness.md', chars: 1480, globs: [], sections: [
      { head: 'The log', laws: ['- **One entry per piece of work**: `.claude/log/YYYY-MM-DD-short-slug.md`, starting with a `# Title` line that says what changed.', '- **CLAUDE.md carries headlines only**: one line per entry linking to its file, newest first.'] },
      { head: 'Evidence', laws: ['- **Say how each claim was established**: `ran:`, `read:` or `not run:`. An unrun check is never reported as passing.'] }] },
    { f: 'tides.md', chars: 2210, globs: ['src/tides/**', 'src/stations/**'], sections: [
      { head: 'Predictions', laws: ['- **Heights are stored in millimetres as integers**: floats drifted by a centimetre over a month of harmonics (log: tide-heights-drift)', '- **A station without a datum is skipped, never defaulted to zero**: a zero datum drew a harbour as dry land (log: dry-harbour)', '- **Times leave the server in UTC with the station\'s zone id beside them**: the client formats; the server never guesses a zone'] },
      { head: 'Station data', laws: ['- **The station list is fetched once a day and cached on disk**: the provider rate-limits at 100 requests an hour', '- **Never trust a station id from the URL**: look it up in the cached list first'] }] },
    { f: 'ui.md', chars: 1320, globs: ['src/app/**', 'src/components/**'], sections: [
      { head: 'Charts', laws: ['- **The tide curve is drawn from at most 288 points**: more made scrolling stutter on older phones', '- **Colour never carries the meaning alone**: high and low water are labelled as well as tinted'] }] },
  ];
  const memories = [
    ['units-preference', 'feedback', 'Metric first, feet as a toggle; never mix both in one label', 'The owner asked for metres by default with a feet toggle in settings.\n\n**Why:** most users are in metric countries; sailors in two markets still think in feet.\n\n**How to apply:** every height label goes through `formatHeight()`; never write a unit by hand.'],
    ['provider-rate-limit', 'reference', 'The tide provider allows 100 requests an hour per key; the key rotates quarterly', 'The provider rate-limits at 100 requests an hour. The dashboard is linked from the team wiki under "Data sources".\n\nThe station list endpoint counts as one request whatever its size.'],
    ['release-rhythm', 'project', 'Releases go out on Tuesdays; a freeze starts Monday noon', 'Releases are cut on Tuesday morning. From Monday noon only fixes land on `main`.\n\n**Why:** the reviewer of the mobile build is only available early in the week.'],
    ['chart-library-decision', 'project', 'Charts are hand-drawn SVG, not a library: bundle size and a custom tide curve', 'We draw the tide curve as plain SVG. Two libraries were tried and dropped: both added over 80 kB and neither could shade the area between two curves the way the design asks.'],
    ['owner-writing-style', 'user', 'Short sentences, no exclamation marks, British spelling in the app copy', 'The owner edits all copy. Short sentences. British spelling ("harbour", "metre"). No exclamation marks anywhere in the interface.'],
  ].map(([name, type, desc, body], i) => ({ f: `${name}.md`, name, desc, type, body, chars: body.length + 160, modified: now - i * 86400 * 3 - 4000 }));
  const skills = [
    ['log', 'project', 'Record a finished piece of work as one log entry plus a headline in CLAUDE.md.'],
    ['rule', 'project', 'Add ONE rule to the right file in .claude/rules.'],
    ['recall', 'project', 'Answer "what happened with X" from the project\'s log, newest first.'],
    ['release-check', 'project', 'Run before a Tuesday release: tests, bundle size, the three smoke pages.'],
    ['review', 'you', 'Review the current diff for correctness before a commit.'],
  ].map(([name, scope, desc]) => ({ name, scope, desc, body: `# /${name}\n\n${desc}\n\n1. Read what changed.\n2. Do the work in small steps.\n3. Say how each claim was checked.`, chars: 420 + desc.length }));
  const titles = ['Tide curve redrawn from 288 points', 'Station search matches harbour names without accents', 'Feet toggle added to settings', 'Dry harbour fixed: a station without a datum is skipped', 'Offline cache for the last seven days of predictions', 'Heights stored as integer millimetres', 'Dependency audit: two packages replaced', 'Moon phase strip under the chart', 'Share link carries the station and the day', 'Rate limit handled with a daily station cache', 'Dark theme contrast pass', 'First-run station picker'];
  const slugOf = (t) => t.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  const log = titles.map((title, i) => { const date = day([0, 1, 1, 3, 4, 6, 9, 10, 13, 17, 22, 30][i]); return { date, title, slug: `${date}-${slugOf(title)}`, pending: false, tags: [] }; });
  const project = { name: 'tidepool', root: 'D:\\projects\\tidepool', dir: '.claude', instructions: 'CLAUDE.md', native: false, kit: false, bare: false };
  const full = {
    project, limit: 120000, sizes: { projClaude: 3840, userClaude: 2150, memoryIndex: 610 }, rules, memories, log,
    ledger: { total: 0, status: {}, repos: {} },
    sessions: { drafts: 0, live: 0, abandoned: 0, recent: [], past: [
      { id: '0a1b2c3d-1111-4222-8333-444455556666', state: 'ended', up: now - 3600 * 5, ask: 'make the tide curve smoother on small screens', files: 0 },
      { id: '1b2c3d4e-1111-4222-8333-444455556666', state: 'ended', up: now - 3600 * 26, ask: 'why does the harbour at station 4412 show as dry', files: 0 },
      { id: '2c3d4e5f-1111-4222-8333-444455556666', state: 'ended', up: now - 3600 * 50, ask: 'add a feet toggle to settings', files: 0 }] },
    registry: [], skills,
    hooks: [
      { scope: 'project', when: 'PostToolUse', on: 'Edit|Write', runs: 'npx prettier --write "$CLAUDE_FILE_PATHS"' },
      { scope: 'project', when: 'PreToolUse', on: 'Bash', runs: 'node scripts/guard-commands.mjs' },
      { scope: 'you', when: 'Stop', on: 'always', runs: 'node ~/.claude/hooks/notify.mjs' }],
    audits: [{ kind: 'doc', title: 'Dependency audit', file: 'dependency-audit.md', date: day(9), where: 'docs', path: 'D:\\projects\\tidepool\\docs\\dependency-audit.md', chars: 5210, lead: 'Two packages were replaced and one pinned. Nothing in the tree is unmaintained.' }],
    probes: null, hot: ['src/tides/predict.ts', 'src/components/TideChart.tsx', 'src/app/settings/page.tsx'], hotkey: 'Ctrl+Shift+Space',
  };
  const bare = { ...full, project: { ...project, name: 'new-idea', root: 'D:\\projects\\new-idea', bare: true }, sizes: { projClaude: 0, userClaude: 2150, memoryIndex: 0 }, rules: [], memories: [], log: [], skills: skills.filter((s) => s.scope === 'you'), hooks: full.hooks.filter((h) => h.scope === 'you'), audits: [], hot: [], sessions: { drafts: 0, live: 0, abandoned: 0, recent: [], past: [] } };
  const empty = { ...bare, empty: true, project: { ...project, name: '', root: '', bare: false }, skills: [], hooks: [] };
  let state = scene === 'welcome' ? { data: empty, names: [], roots: [] } : scene === 'bare' ? { data: bare, names: ['new-idea'], roots: ['D:\\projects\\new-idea'] } : { data: full, names: ['tidepool', 'harbour-api', 'field-notes'], roots: ['D:\\projects\\tidepool', '\\\\wsl.localhost\\Ubuntu\\home\\me\\harbour-api', 'D:\\projects\\field-notes'] };

  const starter = (layout) => { const dir = layout === 'agents' ? '.agents' : '.claude', ins = layout === 'agents' ? 'AGENTS.md' : 'CLAUDE.md'; const f = [[ins, `# ${state.data.project.name}\n\nSay in two or three sentences what this project is.\n\n## Current state\n\n## Roadmap\n\n- [ ] First milestone`], [`${dir}/rules/harness.md`, '# How this project keeps its context\n\n## The log\n- **One entry per piece of work**'], [`${dir}/skills/log/SKILL.md`, '# /log\n\nRecord this session\'s work as ONE entry.'], [`${dir}/skills/rule/SKILL.md`, '# /rule'], [`${dir}/skills/recall/SKILL.md`, '# /recall'], [`${dir}/log/${day(0)}-harness-set-up.md`, '# Harness set up']]; if (layout === 'agents') f.push(['CLAUDE.md', '@AGENTS.md\n']); return { layout, root: state.data.project.root, files: f.map(([path, text]) => ({ path, text, exists: false })) }; };
  const files = [
    { path: 'src/components/TideChart.tsx', staged: '.', work: 'M', add: 42, del: 17 }, { path: 'src/tides/predict.ts', staged: 'M', work: '.', add: 9, del: 3 },
    { path: 'src/tides/predict.test.ts', staged: 'M', work: '.', add: 31, del: 0 }, { path: 'src/app/settings/page.tsx', staged: '.', work: 'M', add: 6, del: 2 }, { path: '.claude/log/' + log[0].slug + '.md', staged: '.', work: '?' }];
  const commits = titles.slice(1, 9).map((t, i) => ({ sha: (i + 10).toString(16).repeat(20).slice(0, 40), short: (i + 10).toString(16).repeat(7).slice(0, 7), who: i % 3 ? 'Sam' : 'Ada', at: `${day(i + 1)} 1${i}:2${i}`, ago: `${i + 1} day${i ? 's' : ''} ago`, subject: `${i % 4 === 3 ? 'fix' : 'feat'}: ${t.toLowerCase()}`, refs: i === 0 ? 'HEAD -> main' : i === 2 ? 'origin/main' : '', unpushed: i < 2 }));
  const diff = '@@ -18,9 +18,12 @@ export function TideChart({ points, unit }: Props) {\n   const width = useWidth(ref);\n-  const step = Math.max(1, Math.floor(points.length / 600));\n-  const shown = points.filter((_, i) => i % step === 0);\n+  // at most 288 points: more made scrolling stutter on older phones\n+  const step = Math.max(1, Math.ceil(points.length / 288));\n+  const shown = points.filter((_, i) => i % step === 0 || i === points.length - 1);\n+  const path = useMemo(() => smooth(shown, width), [shown, width]);\n   return (\n     <svg ref={ref} viewBox={`0 0 ${width} 160`} role="img" aria-label={label}>\n-      <path d={line(shown, width)} className="curve" />\n+      <path d={path} className="curve" />\n       <Labels points={extremes(points)} unit={unit} />\n     </svg>\n';
  const favs = new Set([0]);
  const terms = {};
  const say = (id, text) => emit('term', { id, data: text });
  const script = {
    claude: (id, o) => {
      const ask = o.prompt || 'make the tide curve smoother on small screens'; const where = state.names[o.proj] || state.data.project.name;
      say(id, `\x1b[2m ${where} · main\x1b[0m\r\n\r\n\x1b[1m>\x1b[0m ${ask}\r\n\r\n`);
      const dot = '\x1b[36m●\x1b[0m ';
      const steps = /dry/.test(ask)
        ? [dot + 'Reading src/stations/lookup.ts\r\n', dot + 'Searching the log: dry harbour\r\n', dot + 'Reading .claude/log/' + log[3].slug + '.md\r\n', '\r\nStation 4412 has no datum in the provider\'s list, and an older build drew a missing datum as zero.\r\nThe rule now is to skip such a station. The cached list on this machine is from before that fix.\r\n\r\nRefresh the station cache? \x1b[2m(y/n)\x1b[0m ']
        : [dot + 'Reading src/components/TideChart.tsx\r\n', dot + 'Rule applies: the tide curve is drawn from at most 288 points\r\n', dot + 'Editing src/components/TideChart.tsx \x1b[32m+42\x1b[0m \x1b[31m-17\x1b[0m\r\n', dot + 'Running npm test -- TideChart\r\n  \x1b[32m✓\x1b[0m 14 passed\r\n', '\r\nThe curve now uses 288 points and a smoothed path. Tests pass.\r\n\r\n\x1b[1m>\x1b[0m '];
      steps.forEach((t, i) => setTimeout(() => terms[id] && say(id, t), 250 + i * 350));
    },
    ubuntu: (id) => say(id, 'me@box:/mnt/d/projects/tidepool$ npm run dev\r\n\r\n  ▲ ready on http://localhost:3000\r\n  ✓ compiled in 1.2s\r\n'),
    powershell: (id) => say(id, 'PS D:\\projects\\tidepool> git status --short\r\n M src/components/TideChart.tsx\r\nM  src/tides/predict.ts\r\nPS D:\\projects\\tidepool> '),
  };
  const api = {
    load: () => state.data,
    projects: () => ({ names: state.names, roots: state.roots, favs: state.names.map((_, i) => favs.has(i)), native: state.names.map((n) => n === 'field-notes'), current: 0 }),
    set_project: () => null, set_favorite: ({ i, on }) => { if (on) favs.add(i); else favs.delete(i); return null; }, remove_project: () => null, pick_folder: () => 'D:\\projects\\new-idea',
    machine: () => ({ distros: ['Ubuntu'], windowsClaude: true, windowsClaudeExe: true }),
    inspect_project: ({ folder, runs }) => ({ entry: { name: 'new-idea', root: 'D:\\projects\\new-idea', runs: runs || 'wsl', wslRoot: '/mnt/d/projects/new-idea', distro: 'Ubuntu', claudeHome: '\\\\wsl.localhost\\Ubuntu\\home\\me\\.claude', memoryDir: '\\\\wsl.localhost\\Ubuntu\\home\\me\\.claude\\projects\\-mnt-d-projects-new-idea\\memory', kit: '' }, choices: { wsl: true, windows: true, distro: 'Ubuntu' }, found: { dir: '.claude', claude: false, kit: false, instructions: false, instructionsName: 'CLAUDE.md', rules: 0, log: 0, skills: 0, memory: false, memories: 0, worked: false }, already: false, typed: folder }),
    add_project: () => { state = { data: bare, names: ['new-idea'], roots: ['D:\\projects\\new-idea'] }; return 0; },
    starter_plan: ({ layout }) => starter(layout || 'claude'),
    starter_apply: ({ layout }) => { const pl = starter(layout); state.data = { ...full, project: { ...state.data.project, bare: false, dir: layout === 'agents' ? '.agents' : '.claude', instructions: layout === 'agents' ? 'AGENTS.md' : 'CLAUDE.md' }, rules: rules.slice(0, 1), memories: [], log: [{ date: day(0), title: 'Harness set up', slug: `${day(0)}-harness-set-up`, pending: false, tags: [] }], skills: skills.filter((s) => s.name !== 'release-check'), audits: [], hot: [] }; return { written: pl.files.map((f) => f.path), kept: [] }; },
    read_log: ({ slug }) => `# ${(log.find((e) => e.slug === slug) || { title: 'Harness set up' }).title}\n\n**Date:** ${slug.slice(0, 10)}\n\n## What changed\nThe chart samples the prediction down to 288 points and draws a smoothed path.\n\n## Why\nScrolling stuttered on older phones with the full series.\n\n## Evidence\n- ran: \`npm test -- TideChart\` → 14 passed\n- not run: the real-device check on the oldest supported phone`,
    read_memory: ({ f }) => { const m = memories.find((x) => x.f === f); return `---\nname: ${m.name}\ndescription: ${m.desc}\nmetadata:\n  type: ${m.type}\n---\n\n${m.body}\n`; },
    read_skill: ({ name }) => { const k = skills.find((x) => x.name === name); return `---\nname: ${k.name}\ndescription: ${k.desc}\n---\n\n${k.body}\n`; },
    read_audit: () => '# Dependency audit\n\nTwo packages were replaced and one pinned. Nothing in the tree is unmaintained.\n\n## Replaced\n- the date library, for the platform\'s own `Intl`\n- the chart library, for hand-drawn SVG',
    git_repos: () => [{ name: state.data.project.name, path: state.data.project.root }],
    git_status: ({ repo }) => ({ repo, branch: 'main', upstream: 'origin/main', ahead: 2, behind: 0, files, total: files.length, ms: 41 }),
    git_log: () => commits,
    git_commit: ({ sha }) => { const c = commits.find((x) => x.sha === sha) || commits[0]; return { sha: c.sha, who: c.who, at: c.at, message: c.subject, files: files.slice(0, 3).map((f) => ({ add: String(f.add), del: String(f.del), path: f.path })) }; },
    git_diff: () => ({ text: diff, cut: false }),
    git_tree: ({ dirPath }) => dirPath ? [{ path: `${dirPath}/index.ts`, name: 'index.ts', dir: false }] : ['.claude', 'docs', 'src'].map((n) => ({ path: n, name: n, dir: true })).concat(['CLAUDE.md', 'README.md', 'package.json'].map((n) => ({ path: n, name: n, dir: false }))),
    git_file: () => ({ text: '# tidepool\n\nTide times for small harbours.\n', cut: false }),
    term_check: () => ({ native: false, distro: 'Ubuntu', tmux: true, claude: true }),
    term_list: () => Object.values(terms).filter((t) => t.kind !== 'powershell').map((t) => ({ name: t.name, created: now - 600, kind: t.kind, label: t.label, path: '/mnt/d/projects/tidepool', attached: true })),
    term_open: (o) => { const name = o.kind === 'powershell' ? '' : `maestro-tidepool-${o.kind}-${Date.now()}`; terms[o.id] = { name, kind: o.kind, label: o.label }; setTimeout(() => script[o.kind] && script[o.kind](o.id, o), 60); return name; },
    term_write: ({ id, data }) => { say(id, data === '\r' ? '\r\n' : data); return null; },
    term_close: ({ id }) => { delete terms[id]; emit('term-exit', { id }); return null; },
    autostart_get: () => false, autostart_set: ({ on }) => on,
    note: ({ text }) => { console.warn('[maestro.log]', text); return null; },
  };
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => new Promise((ok, no) => { try { ok(api[cmd] ? api[cmd](args || {}) : null); } catch (e) { no(String(e)); } }) },
    event: { listen: (ev, h) => { (handlers[ev] = handlers[ev] || []).push(h); return Promise.resolve(() => {}); } },
  };
  window.__PREVIEW = { emit, scene };
})();
