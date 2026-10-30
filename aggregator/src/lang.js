// Language normalisation and the audio policy: only Ukrainian or original audio survives.

const ALIASES = {
  uk: ['uk', 'ukr', 'ukrainian', 'українська', 'украинский', 'ua'],
  en: ['en', 'eng', 'english', 'англійська'],
  ru: ['ru', 'rus', 'russian', 'російська', 'русский'],
  fr: ['fr', 'fre', 'fra', 'french'],
  de: ['de', 'ger', 'deu', 'german'],
  it: ['it', 'ita', 'italian'],
  es: ['es', 'spa', 'spanish'],
  ja: ['ja', 'jpn', 'japanese'],
  pl: ['pl', 'pol', 'polish'],
  sv: ['sv', 'swe', 'swedish'],
  zh: ['zh', 'chi', 'zho', 'chinese', 'mandarin'],
  ko: ['ko', 'kor', 'korean'],
  cs: ['cs', 'cze', 'ces', 'czech'],
  pt: ['pt', 'por', 'portuguese'],
  hi: ['hi', 'hin', 'hindi'],
  xx: ['xx', 'zxx', 'silent', 'no linguistic content', 'none'],
};

const LOOKUP = new Map();
for (const [code, names] of Object.entries(ALIASES)) for (const n of names) LOOKUP.set(n, code);

/** Map a free-form language label ("Ukrainian", "ukr", "uk") to ISO 639-1, or null. */
export function normalizeLang(value) {
  if (!value) return null;
  const key = String(value).trim().toLowerCase();
  if (LOOKUP.has(key)) return LOOKUP.get(key);
  return /^[a-z]{2}$/.test(key) ? key : null;
}

/**
 * Decide whether an audio track is allowed.
 * Returns 'ukrainian', 'original', or null (rejected).
 *  - Ukrainian audio is always kept (dub, voice-over or Ukrainian-language original).
 *  - Any other audio is kept only if it matches the film's original language.
 *  - Silent films count as original.
 *  - If the original language is unknown we cannot prove the track is original, so it is rejected.
 */
export function classifyAudio({ audioLanguage, originalLanguage }) {
  const audio = normalizeLang(audioLanguage);
  const original = normalizeLang(originalLanguage);
  if (audio === 'uk') return 'ukrainian';
  if (audio === 'xx' || (audio == null && original === 'xx')) return 'original';
  if (audio && original && audio === original) return 'original';
  return null;
}
