// Serve the page in an ordinary browser with a stand-in for the Rust side, so the interface can be worked on and tested without
// building the app or touching a real project.   node tools/preview/serve.mjs [port]   →   http://localhost:4173/?scene=full
// Scenes: full (a project with a harness and three sessions), bare (a project with nothing set up), welcome (first run).
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, join, normalize, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const ui = join(here, '..', '..', 'ui');
const port = Number(process.argv[2] || 4173);
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8' };

createServer(async (req, res) => {
  try {
    const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (path === '/__preview/stub.js') { res.writeHead(200, { 'content-type': TYPES['.js'] }); return res.end(await readFile(join(here, 'stub.js'))); }
    const file = normalize(join(ui, path === '/' ? 'index.html' : path));
    if (!file.startsWith(ui)) { res.writeHead(403); return res.end(); }
    let body = await readFile(file);
    // the stand-in must exist before the page's own script runs
    if (file.endsWith('index.html')) body = String(body).replace('<script>', '<script src="/__preview/stub.js"></script>\n<script>');
    res.writeHead(200, { 'content-type': TYPES[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' });
    res.end(body);
  } catch (_) { res.writeHead(404); res.end('not found'); }
}).listen(port, '127.0.0.1', () => console.log(`preview: http://localhost:${port}/?scene=full   (also ?scene=bare, ?scene=welcome)`));
