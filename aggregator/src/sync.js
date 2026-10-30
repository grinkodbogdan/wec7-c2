// Pull every source, keep only Ukrainian/original audio, and merge into the local database.
//   npm run sync                 all sources
//   npm run sync -- archive      one source
import { openDb, upsertTitle, upsertOffer } from './db.js';
import { classifyAudio, normalizeLang } from './lang.js';
import { sources } from './sources/index.js';
import { lookup, tmdbEnabled } from './tmdb.js';

// With STRICT_ORIGINAL=1, a track is only kept as "original" when the film's original
// language is known (curated data or TMDB). Otherwise a source's single listed
// language is assumed to be the original and the offer is marked unverified.
const STRICT = process.env.STRICT_ORIGINAL === '1';
const PRUNE_DAYS = Number(process.env.PRUNE_DAYS || 30);

export async function resolveRecord(record, { enrich = lookup } = {}) {
  let originalLanguage = normalizeLang(record.originalLanguage);
  let languageKnown = Boolean(originalLanguage);
  let meta = {};

  if (!languageKnown) {
    meta = (await enrich(record.title, record.year).catch(() => null)) || {};
    originalLanguage = normalizeLang(meta.originalLanguage);
    languageKnown = Boolean(originalLanguage);
  }
  if (!languageKnown && !STRICT) {
    const nonUk = [...new Set(record.offers.map((o) => normalizeLang(o.audioLanguage)).filter((l) => l && l !== 'uk'))];
    if (nonUk.length === 1) originalLanguage = nonUk[0];
  }

  const offers = [];
  let rejected = 0;
  for (const o of record.offers) {
    const audioType = classifyAudio({ audioLanguage: o.audioLanguage, originalLanguage });
    if (!audioType) { rejected++; continue; }
    offers.push({
      ...o,
      audioType,
      audioLanguage: normalizeLang(o.audioLanguage),
      verified: o.verified || audioType === 'ukrainian' || languageKnown,
    });
  }

  return {
    title: {
      title: meta.title || record.title,
      originalTitle: meta.originalTitle || record.originalTitle,
      year: record.year,
      kind: record.kind,
      originalLanguage,
      overview: meta.overview || record.overview,
      poster: meta.poster || record.poster,
    },
    offers,
    rejected,
  };
}

export async function syncSource(db, source) {
  const run = db.prepare("INSERT INTO sync_runs (source, started) VALUES (?, datetime('now'))").run(source.name);
  let accepted = 0, rejected = 0, error = null;
  try {
    for await (const record of source.fetch()) {
      const res = await resolveRecord(record);
      rejected += res.rejected;
      if (!res.offers.length) continue;
      db.exec('BEGIN');
      try {
        const titleId = upsertTitle(db, res.title);
        for (const o of res.offers) upsertOffer(db, titleId, { source: source.name, ...o });
        db.exec('COMMIT');
      } catch (e) {
        db.exec('ROLLBACK');
        throw e;
      }
      accepted += res.offers.length;
    }
    db.prepare(`DELETE FROM offers WHERE source = ? AND last_seen < datetime('now', ?)`).run(source.name, `-${PRUNE_DAYS} days`);
    db.exec('DELETE FROM titles WHERE id NOT IN (SELECT title_id FROM offers)');
  } catch (e) {
    error = e.message;
  }
  db.prepare("UPDATE sync_runs SET finished = datetime('now'), accepted = ?, rejected = ?, error = ? WHERE id = ?")
    .run(accepted, rejected, error, run.lastInsertRowid);
  return { source: source.name, accepted, rejected, error };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const only = process.argv.slice(2);
  const db = openDb();
  console.log(`TMDB enrichment: ${tmdbEnabled ? 'on' : 'off'}; strict original: ${STRICT ? 'on' : 'off'}`);
  for (const source of sources.filter((s) => !only.length || only.includes(s.name))) {
    console.log(`→ ${source.label}…`);
    const r = await syncSource(db, source);
    console.log(`  kept ${r.accepted} offers, rejected ${r.rejected}${r.error ? `, ERROR: ${r.error}` : ''}`);
  }
}
