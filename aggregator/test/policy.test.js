import { test } from 'node:test';
import assert from 'node:assert/strict';
import { classifyAudio, normalizeLang } from '../src/lang.js';
import { openDb, upsertTitle, upsertOffer, searchTitles, matchKey } from '../src/db.js';
import { resolveRecord } from '../src/sync.js';

test('normalizeLang handles labels and codes', () => {
  assert.equal(normalizeLang('Ukrainian'), 'uk');
  assert.equal(normalizeLang('ukr'), 'uk');
  assert.equal(normalizeLang('English'), 'en');
  assert.equal(normalizeLang('Klingon'), null);
});

test('only Ukrainian or original audio is allowed', () => {
  assert.equal(classifyAudio({ audioLanguage: 'uk', originalLanguage: 'en' }), 'ukrainian');
  assert.equal(classifyAudio({ audioLanguage: 'en', originalLanguage: 'en' }), 'original');
  assert.equal(classifyAudio({ audioLanguage: 'ru', originalLanguage: 'en' }), null);
  assert.equal(classifyAudio({ audioLanguage: 'en', originalLanguage: 'it' }), null); // English dub of an Italian film
  assert.equal(classifyAudio({ audioLanguage: 'en', originalLanguage: null }), null);
});

test('resolveRecord drops dubs and uses enrichment for the original language', async () => {
  const record = {
    title: 'Bloody Pit of Horror', year: 1965,
    offers: [{ externalId: 'x', url: 'https://example.org/x', audioLanguage: 'English' }],
  };
  const enrich = async () => ({ originalLanguage: 'it', originalTitle: 'Il boia scarlatto' });
  const res = await resolveRecord(record, { enrich });
  assert.equal(res.offers.length, 0);
  assert.equal(res.rejected, 1);
});

test('resolveRecord keeps Ukrainian and original tracks, rejects Russian dub', async () => {
  const record = {
    title: 'Test', year: 2000, originalLanguage: 'fr',
    offers: [
      { externalId: 'a', url: 'https://example.org/a', audioLanguage: 'ukr' },
      { externalId: 'b', url: 'https://example.org/b', audioLanguage: 'French' },
      { externalId: 'c', url: 'https://example.org/c', audioLanguage: 'Russian' },
    ],
  };
  const res = await resolveRecord(record, { enrich: async () => null });
  assert.deepEqual(res.offers.map((o) => o.audioType), ['ukrainian', 'original']);
  assert.equal(res.rejected, 1);
});

test('same film from two sources merges into one title', () => {
  const db = openDb(':memory:');
  const a = upsertTitle(db, { title: 'Stone Mill (Жернова)', year: 2008 });
  const b = upsertTitle(db, { title: 'Stone Mill', year: 2008 });
  assert.equal(a, b);
  assert.equal(matchKey('Amélie!', 2001), 'amelie|2001');
  upsertOffer(db, a, { source: 's', externalId: '1', url: 'https://example.org', audioType: 'ukrainian' });
  assert.equal(searchTitles(db, { audio: 'ukrainian' }).total, 1);
  assert.equal(searchTitles(db, { audio: 'original' }).total, 0);
});
