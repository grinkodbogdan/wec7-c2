// Hand-curated entries from data/manual/*.json, for legal sources without an API
// (official YouTube channels, free titles on Takflix, festival screenings, etc.).
// Each file is an array of:
// { "title", "originalTitle", "year", "kind", "originalLanguage", "overview", "poster",
//   "offers": [{ "source", "id", "url", "embedUrl", "audioLanguage", "license" }] }

import { readdir, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const DIR = resolve(process.env.MANUAL_DIR || 'data/manual');

export default {
  name: 'manual',
  label: 'Curated',

  async *fetch() {
    let files = [];
    try {
      files = (await readdir(DIR)).filter((f) => f.endsWith('.json'));
    } catch {
      return;
    }
    for (const file of files) {
      const entries = JSON.parse(await readFile(join(DIR, file), 'utf8'));
      for (const e of entries) {
        yield {
          ...e,
          offers: (e.offers || []).map((o) => ({
            source: o.source || 'manual',
            externalId: o.id || o.url,
            url: o.url,
            embedUrl: o.embedUrl,
            audioLanguage: o.audioLanguage,
            license: o.license,
            verified: true, // a person checked the audio track
          })),
        };
      }
    }
  },
};
