// Internet Archive: public-domain and openly licensed feature films and cartoons.
// https://archive.org/advancedsearch.php — no API key needed.

const COLLECTIONS = 'collection:(feature_films OR silent_films OR animationandcartoons OR classic_cartoons OR sciencefiction_horrormovies)';
const PAGE_SIZE = 100;

const QUERIES = [
  // Everything in Ukrainian first, then the most popular films that declare an audio language
  // (untagged items cannot be checked against the audio policy, so they are skipped).
  { q: `${COLLECTIONS} AND language:(ukr OR Ukrainian)`, limit: Infinity },
  { q: `${COLLECTIONS} AND mediatype:movies AND language:*`, limit: Number(process.env.ARCHIVE_LIMIT || 300) },
];

async function search(q, page) {
  const params = new URLSearchParams({ q, rows: String(PAGE_SIZE), page: String(page), output: 'json' });
  for (const f of ['identifier', 'title', 'year', 'language', 'description', 'licenseurl']) params.append('fl[]', f);
  params.append('sort[]', 'downloads desc');
  const res = await fetch(`https://archive.org/advancedsearch.php?${params}`, {
    headers: { 'User-Agent': 'ua-film-aggregator/0.1 (+https://archive.org/about/contact.php)' },
  });
  if (!res.ok) throw new Error(`archive.org HTTP ${res.status}`);
  return (await res.json()).response;
}

const asList = (v) => (v == null ? [] : Array.isArray(v) ? v : [v]);
const firstText = (v) => asList(v).find((s) => typeof s === 'string') || null;

function clean(text, max = 600) {
  if (!text) return null;
  const plain = text.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();
  return plain.length > max ? `${plain.slice(0, max - 1)}…` : plain;
}

export default {
  name: 'archive',
  label: 'Internet Archive',

  async *fetch() {
    const seen = new Set();
    for (const { q, limit } of QUERIES) {
      let count = 0;
      for (let page = 1; count < limit; page++) {
        const { docs, numFound } = await search(q, page);
        if (!docs.length) break;
        for (const d of docs) {
          if (seen.has(d.identifier) || count >= limit) continue;
          seen.add(d.identifier);
          count++;
          const languages = asList(d.language).filter((l) => !/subtit/i.test(l));
          const year = parseInt(d.year, 10) || null;
          yield {
            title: firstText(d.title) || d.identifier,
            year,
            overview: clean(firstText(d.description)),
            poster: `https://archive.org/services/img/${d.identifier}`,
            // archive.org does not record the production language; sync resolves it.
            originalLanguage: null,
            offers: languages.map((lang) => ({
              externalId: d.identifier,
              url: `https://archive.org/details/${d.identifier}`,
              embedUrl: `https://archive.org/embed/${d.identifier}`,
              audioLanguage: lang,
              license: d.licenseurl || 'Internet Archive',
            })),
          };
        }
        if (page * PAGE_SIZE >= numFound) break;
        await new Promise((r) => setTimeout(r, 500)); // be polite
      }
    }
  },
};
