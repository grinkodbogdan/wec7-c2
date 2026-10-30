// Optional TMDB enrichment: original language, original title, Ukrainian overview and poster.
// Enabled when TMDB_TOKEN (v4 read token) or TMDB_API_KEY (v3 key) is set.

const TOKEN = process.env.TMDB_TOKEN;
const KEY = process.env.TMDB_API_KEY;

export const tmdbEnabled = Boolean(TOKEN || KEY);

const cache = new Map();

async function call(path, params) {
  const qs = new URLSearchParams(params);
  if (!TOKEN) qs.set('api_key', KEY);
  const res = await fetch(`https://api.themoviedb.org/3${path}?${qs}`, {
    headers: TOKEN ? { Authorization: `Bearer ${TOKEN}` } : {},
  });
  if (!res.ok) throw new Error(`TMDB HTTP ${res.status}`);
  return res.json();
}

/** Look up a film by title and year; returns null when no confident match. */
export async function lookup(title, year) {
  if (!tmdbEnabled || !title) return null;
  const query = title.replace(/\(.*?\)/g, '').trim();
  const cacheKey = `${query}|${year}`;
  if (cache.has(cacheKey)) return cache.get(cacheKey);

  const { results = [] } = await call('/search/movie', { query, language: 'uk-UA', include_adult: 'false' });
  const match = results.find((r) => {
    const y = parseInt(r.release_date, 10);
    return !year || (y && Math.abs(y - year) <= 1);
  });
  const out = match && {
    originalTitle: match.original_title,
    originalLanguage: match.original_language,
    title: match.title,
    overview: match.overview || null,
    poster: match.poster_path ? `https://image.tmdb.org/t/p/w342${match.poster_path}` : null,
  };
  cache.set(cacheKey, out || null);
  return out || null;
}
