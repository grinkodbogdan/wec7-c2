# Кінопошук UA — film aggregator

A small film aggregator with its own SQLite database. It pulls films from free **legal**
sources, merges duplicates, and keeps **only Ukrainian audio or the film's original audio**.
Dubs into any other language (Russian, an English dub of an Italian film, etc.) are dropped.

No dependencies: Node.js ≥ 22.5 (built-in `node:sqlite`).

```bash
npm run sync     # fill/refresh the database (run it on a cron, e.g. daily)
npm start        # http://localhost:3000
npm test
```

## How the audio rule works

`src/lang.js` → `classifyAudio()`:

| Audio track                           | Result                       |
|---------------------------------------|------------------------------|
| Ukrainian                             | kept as `ukrainian`          |
| Same as the film's original language  | kept as `original`           |
| Silent film                           | kept as `original`           |
| Anything else                         | rejected                     |

The original language comes from curated data first, then from TMDB if a key is set.
Without either, a source's single listed language is assumed to be the original and
the offer is marked **unverified** in the UI. Set `STRICT_ORIGINAL=1` to drop those instead.

## Sources

| Source | File | Notes |
|---|---|---|
| Internet Archive | `src/sources/archive.js` | Public-domain / openly licensed films; everything tagged Ukrainian plus the top `ARCHIVE_LIMIT` (default 300) films that declare an audio language. Playback is embedded. |
| Curated JSON | `src/sources/manual.js` | Drop files in `data/manual/*.json` (see `example.json`) for legal sources without an API: official YouTube channels, free titles on Takflix, Dovzhenko Centre releases, etc. |

To add a source, write a module exporting `{ name, label, async *fetch() }` that yields
records shaped like the curated JSON, and register it in `src/sources/index.js`.
Only add sources that are licensed to distribute what they host.

## Configuration

| Variable | Default | |
|---|---|---|
| `PORT` | `3000` | HTTP port |
| `DB_PATH` | `data/aggregator.db` | SQLite file |
| `TMDB_TOKEN` / `TMDB_API_KEY` | — | Enables TMDB enrichment: original language, Ukrainian titles and overviews, posters |
| `STRICT_ORIGINAL` | off | `1` = only keep "original" tracks whose original language is confirmed |
| `ARCHIVE_LIMIT` | `300` | How many non-Ukrainian Internet Archive films to pull |
| `PRUNE_DAYS` | `30` | Offers not seen in a sync for this many days are removed |

## API

- `GET /api/titles?q=&audio=any|ukrainian|original&page=&perPage=`
- `GET /api/titles/:id` — title plus its offers
- `GET /api/stats`
