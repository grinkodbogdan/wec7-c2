import { DatabaseSync } from 'node:sqlite';
import { mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const DB_PATH = resolve(process.env.DB_PATH || 'data/aggregator.db');

const SCHEMA = `
CREATE TABLE IF NOT EXISTS titles (
  id                INTEGER PRIMARY KEY,
  match_key         TEXT NOT NULL UNIQUE,
  title             TEXT NOT NULL,
  original_title    TEXT,
  year              INTEGER,
  kind              TEXT NOT NULL DEFAULT 'movie',
  original_language TEXT,
  overview          TEXT,
  poster            TEXT,
  created_at        TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS offers (
  id             INTEGER PRIMARY KEY,
  title_id       INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
  source         TEXT NOT NULL,
  external_id    TEXT NOT NULL,
  url            TEXT NOT NULL,
  embed_url      TEXT,
  audio_type     TEXT NOT NULL CHECK (audio_type IN ('ukrainian', 'original')),
  audio_language TEXT,
  verified       INTEGER NOT NULL DEFAULT 0,
  license        TEXT,
  last_seen      TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (source, external_id, audio_type)
);
CREATE INDEX IF NOT EXISTS offers_title ON offers(title_id);
CREATE INDEX IF NOT EXISTS offers_audio ON offers(audio_type);
CREATE TABLE IF NOT EXISTS sync_runs (
  id        INTEGER PRIMARY KEY,
  source    TEXT NOT NULL,
  started   TEXT NOT NULL,
  finished  TEXT,
  accepted  INTEGER DEFAULT 0,
  rejected  INTEGER DEFAULT 0,
  error     TEXT
);
`;

export function openDb(path = DB_PATH) {
  if (path !== ':memory:') mkdirSync(dirname(path), { recursive: true });
  const db = new DatabaseSync(path);
  db.exec('PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;');
  db.exec(SCHEMA);
  return db;
}

/** Key used to merge the same film coming from different sources. */
export function matchKey(title, year) {
  const norm = String(title)
    .normalize('NFKD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .replace(/\(.*?\)/g, ' ')
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim();
  return `${norm}|${year || ''}`;
}

export function upsertTitle(db, t) {
  const key = matchKey(t.originalTitle || t.title, t.year);
  db.prepare(`
    INSERT INTO titles (match_key, title, original_title, year, kind, original_language, overview, poster)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(match_key) DO UPDATE SET
      original_title    = COALESCE(titles.original_title, excluded.original_title),
      original_language = COALESCE(titles.original_language, excluded.original_language),
      overview          = COALESCE(titles.overview, excluded.overview),
      poster            = COALESCE(titles.poster, excluded.poster),
      updated_at        = datetime('now')
  `).run(key, t.title, t.originalTitle ?? null, t.year ?? null, t.kind || 'movie',
    t.originalLanguage ?? null, t.overview ?? null, t.poster ?? null);
  return db.prepare('SELECT id FROM titles WHERE match_key = ?').get(key).id;
}

export function upsertOffer(db, titleId, o) {
  db.prepare(`
    INSERT INTO offers (title_id, source, external_id, url, embed_url, audio_type, audio_language, verified, license)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(source, external_id, audio_type) DO UPDATE SET
      title_id = excluded.title_id, url = excluded.url, embed_url = excluded.embed_url,
      audio_language = excluded.audio_language, verified = excluded.verified,
      license = excluded.license, last_seen = datetime('now')
  `).run(titleId, o.source, o.externalId, o.url, o.embedUrl ?? null, o.audioType,
    o.audioLanguage ?? null, o.verified ? 1 : 0, o.license ?? null);
}

export function searchTitles(db, { q = '', audio = 'any', page = 1, perPage = 24 } = {}) {
  const where = ['EXISTS (SELECT 1 FROM offers o WHERE o.title_id = t.id'
    + (audio === 'ukrainian' || audio === 'original' ? ' AND o.audio_type = $audio' : '') + ')'];
  const params = {};
  if (audio === 'ukrainian' || audio === 'original') params.audio = audio;
  if (q) {
    where.push('(t.title LIKE $q OR t.original_title LIKE $q)');
    params.q = `%${q}%`;
  }
  const sql = where.join(' AND ');
  const total = db.prepare(`SELECT COUNT(*) AS n FROM titles t WHERE ${sql}`).get(params).n;
  const items = db.prepare(`
    SELECT t.id, t.title, t.original_title, t.year, t.kind, t.original_language, t.poster,
      (SELECT GROUP_CONCAT(DISTINCT audio_type) FROM offers WHERE title_id = t.id) AS audio_types
    FROM titles t WHERE ${sql}
    ORDER BY t.updated_at DESC, t.id DESC
    LIMIT $limit OFFSET $offset
  `).all({ ...params, limit: perPage, offset: (page - 1) * perPage });
  for (const it of items) it.audio_types = it.audio_types ? it.audio_types.split(',') : [];
  return { total, page, perPage, items };
}

export function getTitle(db, id) {
  const title = db.prepare('SELECT * FROM titles WHERE id = ?').get(id);
  if (!title) return null;
  title.offers = db.prepare(`
    SELECT source, url, embed_url, audio_type, audio_language, verified, license, last_seen
    FROM offers WHERE title_id = ? ORDER BY audio_type = 'ukrainian' DESC, verified DESC
  `).all(id);
  return title;
}

export function stats(db) {
  return {
    titles: db.prepare('SELECT COUNT(*) AS n FROM titles').get().n,
    ukrainian: db.prepare("SELECT COUNT(DISTINCT title_id) AS n FROM offers WHERE audio_type = 'ukrainian'").get().n,
    original: db.prepare("SELECT COUNT(DISTINCT title_id) AS n FROM offers WHERE audio_type = 'original'").get().n,
    sources: db.prepare('SELECT source, COUNT(*) AS offers FROM offers GROUP BY source').all(),
    lastRuns: db.prepare('SELECT source, finished, accepted, rejected, error FROM sync_runs ORDER BY id DESC LIMIT 10').all(),
  };
}
