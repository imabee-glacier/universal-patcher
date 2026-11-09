import { language, t, translatePage, localized } from './i18n.js';
// Presentation only. Release and installer URLs come from the promoted catalog.
// The empty checked-in catalog deliberately advertises no supported platform.
const $ = (selector) => document.querySelector(selector);
const all = (selector) => [...document.querySelectorAll(selector)];
let catalog = { downloads: [], packs: [], releases: [] };
let selectedOS = '';
let filter = 'all';
let catalogFailed = false;
const osNames = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
const kinds = { expansion: 'Expansion pack', game: 'Game pack', stuff: 'Stuff pack', kit: 'Kit' };

export function safeURL(value) {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}
export function validCatalog(data) {
  if (!data || data.schema !== 1 || !['downloads', 'packs', 'releases'].every(key => Array.isArray(data[key]))) return false;
  const text = value => typeof value === 'string' && value.trim().length > 0;
  if (data.published_at !== null && !Number.isFinite(Date.parse(data.published_at))) return false;
  if ((data.downloads.length || data.packs.length || data.releases.length) && !data.published_at) return false;
  const combinations = new Set();
  return data.downloads.every(item => {
    if (!item || !osNames[item.os] || !['x86_64', 'aarch64'].includes(item.arch) || typeof item.verified !== 'boolean' || !text(item.version)) return false;
    const key = `${item.os}/${item.arch}`;
    if (combinations.has(key)) return false;
    combinations.add(key);
    if (item.verified && !safeURL(item.url)) return false;
    if (item.install_script !== undefined) {
      const url = safeURL(item.install_script);
      // Only canonical, argument-free HTTPS script URLs can become shell commands.
      if (!url || !/^https:\/\/[a-zA-Z0-9.-]+(?::[0-9]+)?\/[a-zA-Z0-9/_-]+\.(sh|ps1)$/.test(item.install_script)) return false;
      if (!item.install_script.endsWith(item.os === 'windows' ? '.ps1' : '.sh')) return false;
    }
    return true;
  }) && data.packs.every(item => item && text(item.id) && text(item.name) && kinds[item.kind] && Array.isArray(item.platforms) && item.platforms.length > 0 && item.platforms.every(os => osNames[os]) && Number.isSafeInteger(item.download_size) && item.download_size >= 0)
    && new Set(data.packs.map(item => item.id)).size === data.packs.length
    && data.releases.every(item => item && text(item.version) && Number.isFinite(Date.parse(item.date)) && Array.isArray(item.notes) && item.notes.every(text));
}
function node(tag, className, text) {
  const result = document.createElement(tag);
  if (className) result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}
function renderDownload() {
  const arch = $('#architecture').value;
  const release = catalog.downloads.find(item => item.os === selectedOS && item.arch === arch && item.verified);
  const link = $('#download-link');
  link.hidden = !release;
  link.removeAttribute('href');
  $('#download-unavailable').hidden = Boolean(release);
  $('#install-command').hidden = true;
  $('#command-text').textContent = '';
  $('#copy-status').textContent = '';
  if (release) {
    link.href = safeURL(release.url);
    link.textContent = t('Download for {os} ↗', { os: osNames[selectedOS] });
    $('#download-state').textContent = `${t('Version {version}', { version: release.version })} · ${arch === 'x86_64' ? 'x64' : 'ARM64'}`;
    $('#download-reason').textContent = t('A verified release is available for this platform.');
    if (release.install_script) {
      $('#install-command').hidden = false;
      $('#command-text').textContent = selectedOS === 'windows' ? `irm '${release.install_script}' | iex` : `curl -fsSL '${release.install_script}' | bash`;
    }
  } else {
    $('#download-state').textContent = catalogFailed ? t('Release information is unavailable.') : selectedOS ? `${osNames[selectedOS]}${arch ? ` · ${arch === 'x86_64' ? 'x64' : 'ARM64'}` : ''}` : t('A little more care before we launch.');
    $('#download-reason').textContent = catalogFailed ? t('Please try again later. Downloads stay unavailable until the catalog can be checked.') : !selectedOS ? t('Public installers are not available yet. Choose a platform to check its availability.') : !arch ? t('Choose your architecture to check for a verified installer.') : t('No verified public installer is published for this selection yet.');
  }
  const hasRelease = catalog.downloads.some(item => item.verified);
  $('.availability').textContent = hasRelease ? t('Platform availability and installation options below') : t('In development · Check platform availability below');
  $('.panel-heading .outline-tag').textContent = hasRelease ? t('Releases') : t('Preview');
  $('.questions details:first-child p').textContent = hasRelease ? t('Published platforms: {platforms}. Use the download selector to check each architecture. Other platform variants remain unavailable until separately verified.', { platforms: [...new Set(catalog.downloads.filter(item => item.verified).map(item => osNames[item.os]))].join(', ') }) : t('Windows, Linux, and native macOS have separate verification requirements. No public platform release is currently listed. The download selector will show availability for each OS and architecture when verified releases are published.');
}
function renderPacks() {
  const target = $('#pack-results');
  const query = $('#pack-search').value.trim().toLocaleLowerCase();
  if (!catalog.packs.length && !query && filter === 'all' && !catalogFailed) {
    target.replaceChildren(emptyCatalog.cloneNode(true));
    translatePage();
    return;
  }
  target.replaceChildren();
  const packs = catalog.packs.filter(pack => (filter === 'all' || (filter === 'stuff' ? ['stuff', 'kit'].includes(pack.kind) : pack.kind === filter)) && `${localized(pack, 'name')} ${pack.name} ${pack.id}`.toLocaleLowerCase(language).includes(query));
  if (!packs.length) {
    target.append(node('p', 'no-results', catalogFailed ? t('The catalog could not be loaded. Please try again later.') : catalog.packs.length ? t('No packs match this search. Try another name or category.') : t('No packs have been published yet. The catalog will appear after the first release.')));
    return;
  }
  const grid = node('div', 'pack-grid');
  for (const pack of packs) {
    const card = node('article', 'pack-card');
    card.append(node('span', 'outline-tag', t(kinds[pack.kind])), node('h3', '', localized(pack, 'name')), node('p', '', pack.platforms.map(os => osNames[os]).join(' · ')), node('p', '', t('{size} GB download', { size: new Intl.NumberFormat(language, { maximumFractionDigits: 1 }).format(pack.download_size / 1024 ** 3) })));
    card.querySelector('h3').dataset.catalogCopy = '';
    grid.append(card);
  }
  target.append(grid);
}
function renderReleases() {
  if (!catalog.releases.length && !catalogFailed) return;
  const target = $('#release-notes');
  target.replaceChildren();
  if (catalogFailed) {
    target.append(node('h3', '', t('Release notes are unavailable.')), node('p', '', t('Please check again when the published catalog can be reached.')));
    return;
  }
  for (const release of [...catalog.releases].sort((a, b) => Date.parse(b.date) - Date.parse(a.date))) {
    const article = node('article');
    article.append(node('span', 'outline-tag', new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeZone: 'UTC' }).format(new Date(release.date))), node('h3', '', t('Version {version}', { version: release.version })));
    for (const note of localized(release, 'notes')) {
      const copy = node('p', '', note);
      copy.dataset.catalogCopy = '';
      article.append(copy);
    }
    target.append(article);
  }
}
async function getJSON(path) {
  const response = await fetch(path, { cache: 'no-store', signal: AbortSignal.timeout(8000) });
  if (!response.ok) throw new Error('Data unavailable');
  return response.json();
}
const emptyCatalog = $('.empty-catalog').cloneNode(true);
let suggestedOS = '';
let health = null;
function renderHint() {
  $('#platform-hint').textContent = suggestedOS ? t('{os} suggested by your browser. Confirm your architecture below.', { os: osNames[suggestedOS] }) : t('Choose your operating system and architecture.');
}
translatePage();
all('[data-os]').forEach(button => button.addEventListener('click', () => {
  selectedOS = button.dataset.os;
  all('[data-os]').forEach(option => option.setAttribute('aria-pressed', String(option === button)));
  renderDownload();
}));
$('#architecture').addEventListener('change', renderDownload);
all('[data-filter]').forEach(button => button.addEventListener('click', () => {
  filter = button.dataset.filter;
  all('[data-filter]').forEach(option => {
    option.setAttribute('aria-pressed', String(option === button));
    option.classList.toggle('active', option === button);
  });
  renderPacks();
}));
$('#pack-search').addEventListener('input', renderPacks);
$('#copy-command').addEventListener('click', async () => {
  try {
    await navigator.clipboard.writeText($('#command-text').textContent);
    $('#copy-status').textContent = t('Installation command copied.');
  } catch {
    $('#copy-status').textContent = t('Copy is unavailable. Select the command and copy it manually.');
  }
});
// Suggest an OS only when its reported platform is unambiguous; never infer CPU architecture.
const reportedPlatform = navigator.userAgentData?.platform || navigator.platform || '';
const tablet = /iPad|iPhone|Android/i.test(navigator.userAgent) || (/Mac/.test(reportedPlatform) && navigator.maxTouchPoints > 1);
if (!tablet) {
  const detected = /Win/i.test(reportedPlatform) ? 'windows' : /Mac/i.test(reportedPlatform) ? 'macos' : /Linux/i.test(reportedPlatform) ? 'linux' : '';
  if (detected) {
    $(`[data-os="${detected}"]`).click();
    suggestedOS = detected;
    renderHint();
  }
}
getJSON('data/catalog.json').then(data => {
  if (!validCatalog(data)) throw new Error('Invalid catalog');
  catalog = data;
}).catch(() => { catalogFailed = true; }).finally(() => {
  renderDownload(); renderPacks(); renderReleases();
});
function renderHealth() {
  $('#service-status').textContent = t('Service status unknown · No health check available');
  const data = health;
  if (!data) return;
  const date = Date.parse(data.checked_at);
  const age = Date.now() - date;
  if (data.schema !== 1 || !['operational', 'degraded', 'unavailable'].includes(data.status) || !Number.isFinite(date) || age < -60000) return;
  const checked = new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(date));
  const status = age > 15 * 60 * 1000 ? t('Status unknown · Last check is stale') : { operational: t('Services operational'), degraded: t('Service disruption'), unavailable: t('Services unavailable') }[data.status];
  $('#service-status').textContent = t('{status} · Checked {date}', { status, date: checked });
}
getJSON('data/health.json').then(data => { health = data; renderHealth(); }).catch(() => { /* Keep the explicit unknown state; do not infer health from release data. */ });
