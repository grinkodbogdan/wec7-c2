// Source registry. To add a source, write a module exporting
// { name, label, async *fetch() } that yields normalised records (see manual.js)
// and register it here. Only add sources that are licensed to distribute what they host.
import archive from './archive.js';
import manual from './manual.js';

export const sources = [manual, archive];
