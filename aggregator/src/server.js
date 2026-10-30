import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, resolve } from 'node:path';
import { openDb, searchTitles, getTitle, stats } from './db.js';

const PORT = Number(process.env.PORT || 3000);
const PUBLIC = resolve('public');
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };

const db = openDb();

function json(res, status, body) {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
  res.end(JSON.stringify(body));
}

async function serveStatic(res, pathname) {
  const file = normalize(join(PUBLIC, pathname === '/' ? 'index.html' : pathname));
  if (!file.startsWith(PUBLIC)) return json(res, 403, { error: 'forbidden' });
  try {
    const body = await readFile(file);
    res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
    res.end(body);
  } catch {
    json(res, 404, { error: 'not found' });
  }
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const { pathname, searchParams: p } = url;
  try {
    if (pathname === '/api/titles') {
      return json(res, 200, searchTitles(db, {
        q: (p.get('q') || '').trim().slice(0, 100),
        audio: p.get('audio') || 'any',
        page: Math.max(1, parseInt(p.get('page'), 10) || 1),
        perPage: Math.min(60, Math.max(1, parseInt(p.get('perPage'), 10) || 24)),
      }));
    }
    const m = pathname.match(/^\/api\/titles\/(\d+)$/);
    if (m) {
      const title = getTitle(db, Number(m[1]));
      return title ? json(res, 200, title) : json(res, 404, { error: 'not found' });
    }
    if (pathname === '/api/stats') return json(res, 200, stats(db));
    return serveStatic(res, pathname);
  } catch (e) {
    console.error(e);
    json(res, 500, { error: 'internal error' });
  }
});

server.listen(PORT, () => console.log(`http://localhost:${PORT}`));
