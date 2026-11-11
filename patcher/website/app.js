import { language, t, translatePage, localized } from './i18n.js';
// Presentation only. Release and installer URLs come from the promoted catalog.
// Download availability comes only from the promoted catalog. The reference pack list
// describes recognized packs, never local installation state or download authorization.
const $ = (selector) => document.querySelector(selector);
const all = (selector) => [...document.querySelectorAll(selector)];
let catalog = { downloads: [], packs: [], releases: [] };
let selectedOS = '';
let catalogFailed = false;
const osNames = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
// Where The Sims 4 runs: Windows and Linux on x64 only, macOS on Intel and Apple silicon.
// Nothing else is listed, offered, or accepted from the catalog.
const gamePlatforms = { windows: ['x86_64'], linux: ['x86_64'], macos: ['x86_64', 'aarch64'] };
const archNames = { x86_64: 'x64', aarch64: 'ARM64' };
const macNames = { x86_64: 'Intel', aarch64: 'Apple silicon' };
// Windows and Linux have one architecture; only a Mac needs the question.
const archFor = os => gamePlatforms[os]?.length === 1 ? gamePlatforms[os][0] : $('#architecture').value;
const platformName = (os, arch) => os === 'macos' ? `macOS · ${macNames[arch] ?? ''}`.replace(/ · $/, '') : `${osNames[os]} · ${archNames[arch]}`;

export function safeURL(value) {
  if (typeof value !== 'string') return null;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}
// Catalogs written before the field was renamed call it `verified`; it meant the same thing.
export function offered(item) { return item.offered ?? item.verified; }
export function validCatalog(data) {
  if (!data || data.schema !== 1 || !['downloads', 'packs', 'releases'].every(key => Array.isArray(data[key]))) return false;
  const text = value => typeof value === 'string' && value.trim().length > 0;
  if (data.published_at !== null && !Number.isFinite(Date.parse(data.published_at))) return false;
  if ((data.downloads.length || data.releases.length) && !data.published_at) return false;
  const combinations = new Set();
  return data.downloads.every(item => {
    if (!item || !gamePlatforms[item.os]?.includes(item.arch) || typeof offered(item) !== 'boolean' || !text(item.version)) return false;
    if (item.offered !== undefined && item.verified !== undefined && item.offered !== item.verified) return false;
    const key = `${item.os}/${item.arch}`;
    if (combinations.has(key)) return false;
    combinations.add(key);
    if (offered(item) && !safeURL(item.url)) return false;
    if (item.install_script !== undefined) {
      const url = safeURL(item.install_script);
      // Only canonical, argument-free HTTPS script URLs can become shell commands.
      if (!url || !/^https:\/\/[a-zA-Z0-9.-]+(?::[0-9]+)?\/[a-zA-Z0-9/_-]+\.(sh|ps1)$/.test(item.install_script)) return false;
      if (!item.install_script.endsWith(item.os === 'windows' ? '.ps1' : '.sh')) return false;
    }
    return true;
  }) && data.packs.every(item => item && ['id', 'name', 'kind'].every(key => text(item[key]))) && data.releases.every(item => item && text(item.version) && Number.isFinite(Date.parse(item.date)) && Array.isArray(item.notes) && item.notes.every(text));
}
function node(tag, className, text) {
  const result = document.createElement(tag);
  if (className) result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}
const platformList = items => [...new Set(items.map(item => `${osNames[item.os]} (${(item.os === 'macos' ? macNames : archNames)[item.arch]})`))].join(', ');
function renderDownload() {
  $('.architecture').hidden = selectedOS !== 'macos';
  const arch = selectedOS ? archFor(selectedOS) : '';
  const release = catalog.downloads.find(item => item.os === selectedOS && item.arch === arch && offered(item));
  const link = $('#download-link');
  link.hidden = !release;
  link.removeAttribute('href');
  $('#download-unavailable').hidden = Boolean(release);
  // Say what is missing; "not available" only once a complete selection has no release.
  $('#unavailable-label').textContent = !selectedOS ? t('Choose your system first') : !arch ? t('Choose your Mac') : t('Not available for this system');
  $('#install-command').hidden = true;
  $('#command-text').textContent = '';
  $('#copy-status').textContent = '';
  if (release) {
    link.href = safeURL(release.url);
    link.textContent = t('Download for {os}', { os: osNames[selectedOS] });
    $('#download-state').textContent = `${t('Version {version}', { version: release.version })} · ${platformName(selectedOS, arch)}`;
    // Windows builds carry no code signature yet, so Windows warns about an unknown publisher.
    $('#download-reason').textContent = selectedOS === 'windows'
      ? `${t('Available for {platform}.', { platform: platformList([release]) })} ${t('Windows may warn that the publisher is unknown, because the installer is not code-signed yet.')}`
      : t('Available for {platform}.', { platform: platformList([release]) });
    if (release.install_script) {
      $('#install-command').hidden = false;
      $('#command-text').textContent = selectedOS === 'windows' ? `irm '${release.install_script}' | iex` : `curl -fsSL '${release.install_script}' | bash`;
    }
  } else {
    $('#download-state').textContent = catalogFailed ? t('Release information is unavailable.') : selectedOS ? (arch ? platformName(selectedOS, arch) : osNames[selectedOS]) : t('Choose your system.');
    $('#download-reason').textContent = catalogFailed ? t('Please try again later. Downloads stay unavailable until the catalog can be checked.') : !selectedOS ? t('Choose a system to see whether an installer is offered for it.') : !arch ? t('Choose Intel or Apple silicon to see whether an installer is offered.') : t('No installer is offered for this system yet.');
  }
  const available = catalog.downloads.filter(offered);
  $('.availability').textContent = available.length ? t('Available for {platforms}. In development.', { platforms: platformList(available) }) : t('In development. Check platform availability below.');
  $('.questions details:first-child p').textContent = available.length ? t('Available now: {platforms}. Universal Patcher is offered only where The Sims 4 runs: Windows and Linux on x64 computers, and macOS on Intel and Apple silicon. Other systems appear here when an installer for them is published.', { platforms: platformList(available) }) : t('Universal Patcher is offered only where The Sims 4 runs: Windows and Linux on x64 computers, and macOS on Intel and Apple silicon. No platform is offered yet.');
}
// Published notes are never rewritten; a note found to be wrong gets a correction shown beside it.
const corrections = {
  '0.1.0': 'Correction: the patcher does not update on its own. Choose Check for patcher update in Settings, or run universal-patcher update.',
};
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
    if (Object.hasOwn(corrections, release.version)) article.append(node('p', 'correction', t(corrections[release.version])));
    target.append(article);
  }
}
async function getJSON(path) {
  const response = await fetch(path, { cache: 'no-store', signal: AbortSignal.timeout(8000) });
  if (!response.ok) throw new Error('Data unavailable');
  return response.json();
}
let suggestedOS = '';
let suggestedArch = '';
let health = null;
function renderHint() {
  $('#platform-hint').textContent = suggestedOS === 'macos' && suggestedArch
    ? t('{os} detected by your browser. You can change it below.', { os: platformName('macos', suggestedArch) })
    : suggestedOS === 'macos' ? t('macOS suggested by your browser. Choose Intel or Apple silicon below.')
    : suggestedOS ? t('{os} detected by your browser. You can change it below.', { os: osNames[suggestedOS] }) : t('Choose your operating system.');
}
translatePage();
all('[data-os]').forEach(button => button.addEventListener('click', () => {
  selectedOS = button.dataset.os;
  all('[data-os]').forEach(option => option.setAttribute('aria-pressed', String(option === button)));
  renderDownload();
}));
$('#architecture').addEventListener('change', renderDownload);
$('#copy-command').addEventListener('click', async () => {
  try {
    await navigator.clipboard.writeText($('#command-text').textContent);
    $('#copy-status').textContent = t('Installation command copied.');
  } catch {
    $('#copy-status').textContent = t('Copy is unavailable. Select the command and copy it manually.');
  }
});
// Suggest an OS only when its reported platform is unambiguous; a Mac's processor is asked or reported, never guessed.
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
// Only a Mac has a choice to make. Chromium-based browsers report the CPU through client hints;
// elsewhere it stays manual. A reported value is detection, not a guess, and a choice the
// visitor already made is kept.
if (suggestedOS === 'macos' && navigator.userAgentData?.getHighEntropyValues) {
  navigator.userAgentData.getHighEntropyValues(['architecture', 'bitness']).then(({ architecture, bitness }) => {
    const detected = bitness === '64' ? { x86: 'x86_64', arm: 'aarch64' }[architecture] : undefined;
    if (!detected || $('#architecture').value) return;
    suggestedArch = detected;
    $('#architecture').value = detected;
    renderHint();
    renderDownload();
  }).catch(() => { /* Keep the architecture choice manual. */ });
}
getJSON('data/catalog.json').then(data => {
  if (!validCatalog(data)) throw new Error('Invalid catalog');
  catalog = data;
}).catch(() => { catalogFailed = true; }).finally(() => {
  renderDownload(); renderReleases(); renderListings();
});
// health.json records requests to the download service (front, install scripts, latest
// patcher version, patcher metadata expiry), so the footer names that service and nothing more.
function renderHealth() {
  $('#service-status').textContent = t('Download service status unknown · No check available');
  const data = health;
  if (!data) return;
  const date = Date.parse(data.checked_at);
  const age = Date.now() - date;
  if (data.schema !== 1 || !['operational', 'degraded', 'unavailable'].includes(data.status) || !Number.isFinite(date) || age < -60000) return;
  const checked = new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(date));
  const status = age > 15 * 60 * 1000 ? t('Download service status unknown · Last check is stale') : { operational: t('Download service reachable'), degraded: t('Download service: some checks failed'), unavailable: t('Download service not reachable') }[data.status];
  $('#service-status').textContent = t('{status} · Checked {date}', { status, date: checked });
}
getJSON('data/health.json').then(data => { health = data; renderHealth(); }).catch(() => { /* Keep the explicit unknown state; do not infer health from release data. */ });

let referencePacks = [];
function renderListings() {
  const packs = catalog.packs.length ? catalog.packs : referencePacks;
  const query = $('#pack-search').value.trim().toLocaleLowerCase(language);
  const matches = packs.filter(pack => `${pack.id} ${pack.name} ${pack.kind}`.toLocaleLowerCase(language).includes(query));
  $('#pack-list').replaceChildren();
  $('#pack-count').textContent = packs.length ? t('{shown} of {total} packs', { shown: matches.length, total: packs.length }) : t('Pack catalog unavailable. Please try again later.');
  for (const pack of matches) {
    const card = node('article', 'pack-card');
    card.dataset.catalogCopy = '';
    card.append(node('h3', '', pack.name), node('p', '', `${pack.id} · ${pack.kind}`));
    if (Number.isSafeInteger(pack.download_size) && pack.download_size >= 0) card.append(node('p', '', `${new Intl.NumberFormat(language).format(pack.download_size)} bytes`));
    $('#pack-list').append(card);
  }
  $('#platform-list').replaceChildren();
  for (const [os, arches] of Object.entries(gamePlatforms)) for (const arch of arches) {
    const item = catalog.downloads.find(item => item.os === os && item.arch === arch);
    const row = node('p');
    const label = node('span', '', `${platformName(os, arch)} — `);
    label.dataset.catalogCopy = '';
    row.append(label);
    if (item && offered(item)) {
      const link = node('a', '', `${t('Download patcher')} · ${item.version}`);
      link.href = safeURL(item.url);
      row.append(link);
    } else row.append(node('span', '', t(catalogFailed ? 'Release information is unavailable.' : 'No installer is offered for this system yet.')));
    $('#platform-list').append(row);
  }
}
$('#pack-search').addEventListener('input', renderListings);
getJSON('data/packs.json').then(data => {
  if (data.schema !== 1 || !Array.isArray(data.packs) || !data.packs.every(pack => pack && ['id', 'name', 'kind'].every(key => typeof pack[key] === 'string'))) throw new Error('Invalid pack reference');
  referencePacks = data.packs;
}).catch(() => {}).finally(renderListings);
