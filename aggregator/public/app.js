const state = { q: '', audio: 'any', page: 1, perPage: 24, total: 0 };
const $ = (s) => document.querySelector(s);

const LANGS = { uk: 'українська', en: 'англійська', fr: 'французька', de: 'німецька', it: 'італійська',
  es: 'іспанська', ja: 'японська', pl: 'польська', sv: 'шведська', xx: 'без звуку', ru: 'російська' };
const audioLabel = (o) => (o.audio_type === 'ukrainian'
  ? '🇺🇦 Українська'
  : `Оригінал (${LANGS[o.audio_language] || o.audio_language || '—'})`);

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}

function safeUrl(u) {
  try {
    const url = new URL(u);
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : '#';
  } catch { return '#'; }
}

async function load() {
  const params = new URLSearchParams({ q: state.q, audio: state.audio, page: state.page, perPage: state.perPage });
  const data = await fetch(`/api/titles?${params}`).then((r) => r.json());
  state.total = data.total;
  $('#summary').textContent = data.total ? `Знайдено: ${data.total}` : 'Нічого не знайдено. Запустіть синхронізацію: npm run sync';
  $('#grid').innerHTML = data.items.map((t) => `
    <button class="card" data-id="${t.id}">
      <div class="poster">${t.poster ? `<img loading="lazy" src="${esc(safeUrl(t.poster))}" alt="">` : ''}</div>
      <div class="meta">
        <strong>${esc(t.title)}</strong>
        <span class="muted">${esc(t.year || '')}</span>
        <span class="badges">${t.audio_types.map((a) => `<i class="badge ${a}">${a === 'ukrainian' ? 'UA' : 'ORIG'}</i>`).join('')}</span>
      </div>
    </button>`).join('');
  const pages = Math.max(1, Math.ceil(data.total / state.perPage));
  $('#page').textContent = `${state.page} / ${pages}`;
  $('#prev').disabled = state.page <= 1;
  $('#next').disabled = state.page >= pages;
}

async function openTitle(id) {
  const t = await fetch(`/api/titles/${id}`).then((r) => r.json());
  const player = t.offers.find((o) => o.embed_url);
  $('#detail-body').innerHTML = `
    <h2>${esc(t.title)} <span class="muted">${esc(t.year || '')}</span></h2>
    ${t.original_title && t.original_title !== t.title ? `<p class="muted">${esc(t.original_title)}</p>` : ''}
    ${player ? `<div class="player"><iframe src="${esc(safeUrl(player.embed_url))}" allowfullscreen loading="lazy"></iframe></div>` : ''}
    ${t.overview ? `<p>${esc(t.overview)}</p>` : ''}
    <h3>Де дивитися</h3>
    <ul class="offers">${t.offers.map((o) => `
      <li>
        <span class="badge ${o.audio_type}">${esc(audioLabel(o))}</span>
        ${o.verified ? '' : '<span class="muted" title="Мова оригіналу не підтверджена">неперевірено</span>'}
        <a href="${esc(safeUrl(o.url))}" target="_blank" rel="noopener">${esc(o.source)} ↗</a>
      </li>`).join('')}
    </ul>`;
  $('#detail').showModal();
}

$('#search').addEventListener('submit', (e) => { e.preventDefault(); });
let timer;
$('#q').addEventListener('input', (e) => {
  clearTimeout(timer);
  timer = setTimeout(() => { state.q = e.target.value; state.page = 1; load(); }, 250);
});
document.querySelectorAll('.filters button').forEach((b) => b.addEventListener('click', () => {
  document.querySelectorAll('.filters button').forEach((x) => x.setAttribute('aria-pressed', x === b));
  state.audio = b.dataset.audio; state.page = 1; load();
}));
$('#prev').addEventListener('click', () => { state.page--; load(); scrollTo(0, 0); });
$('#next').addEventListener('click', () => { state.page++; load(); scrollTo(0, 0); });
$('#grid').addEventListener('click', (e) => { const c = e.target.closest('.card'); if (c) openTitle(c.dataset.id); });
$('#detail .close').addEventListener('click', () => $('#detail').close());
$('#detail').addEventListener('close', () => { $('#detail-body').innerHTML = ''; });

fetch('/api/stats').then((r) => r.json()).then((s) => {
  $('#stats').textContent = `· ${s.titles} фільмів · 🇺🇦 ${s.ukrainian} · оригінал ${s.original}`;
});
load();
