import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { JSDOM } from 'jsdom';

const root = new URL('../', import.meta.url);
const html = await readFile(new URL('index.html', root), 'utf8');
const translations = (await readFile(new URL('i18n.js', root), 'utf8')).replace(/export /g, '');
const source = translations + '\n' + (await readFile(new URL('app.js', root), 'utf8')).replace(/^import .*?;\n/, '').replace(/export function /g, 'function ');
const empty = JSON.parse(await readFile(new URL('data/catalog.json', root), 'utf8'));
const fixture = {
  schema: 1, published_at: '2026-11-09T12:00:00Z',
  downloads: [{ os: 'linux', arch: 'x86_64', verified: true, version: 'test-1', url: 'https://downloads.example.test/linux.tar.gz', install_script: 'https://downloads.example.test/install.sh' }],
  packs: [
    { id: 'test-expansion', name: 'Fixture expansion', kind: 'expansion', platforms: ['linux'], download_size: 1073741824 },
    { id: 'test-kit', name: '<img src=x onerror=alert(1)>', kind: 'kit', platforms: ['linux'], download_size: 4096 },
  ],
  releases: [{ version: 'test-1', date: '2026-11-09', notes: ['Fixture release note.'] }],
};
async function mount({ catalog = empty, health = { schema: 1, status: 'unknown', checked_at: null }, fail = false, platform = '', clipboard, languages = ['en-US'], savedLanguage, blockedStorage = false } = {}) {
  const dom = new JSDOM(html, { url: 'https://website.example.test/', runScripts: 'outside-only' });
  Object.defineProperty(dom.window.navigator, 'platform', { value: platform });
  Object.defineProperty(dom.window.navigator, 'languages', { value: languages });
  Object.defineProperty(dom.window.navigator, 'language', { value: languages[0] });
  if (savedLanguage) dom.window.localStorage.setItem('patcher-website-language', savedLanguage);
  if (blockedStorage) Object.defineProperty(dom.window, 'localStorage', { get() { throw new Error('blocked'); } });
  if (clipboard) Object.defineProperty(dom.window.navigator, 'clipboard', { value: clipboard });
  dom.window.fetch = async (path) => {
    if (fail) throw new Error('offline');
    return { ok: true, json: async () => structuredClone(path.includes('catalog') ? catalog : health) };
  };
  dom.window.eval(source);
  await new Promise(resolve => setImmediate(resolve));
  const doc = dom.window.document;
  return { dom, doc, $: selector => doc.querySelector(selector), all: selector => [...doc.querySelectorAll(selector)] };
}
function selectPlatform(view, os = 'linux', arch = 'x86_64') {
  view.$(`[data-os="${os}"]`).click();
  view.$('#architecture').value = arch;
  view.$('#architecture').dispatchEvent(new view.dom.window.Event('change'));
}

test('all local asset URLs and in-page links resolve; IDs are unique', async () => {
  const dom = new JSDOM(html);
  const doc = dom.window.document;
  const ids = [...doc.querySelectorAll('[id]')].map(el => el.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const el of doc.querySelectorAll('[src],link[href],a[href]')) {
    const url = el.getAttribute('src') ?? el.getAttribute('href');
    if (url.startsWith('#')) { if (url.length > 1) assert.ok(doc.getElementById(url.slice(1)), url); }
    else if (!url.includes('://')) assert.ok((await readFile(new URL(url, root))).byteLength > 0, url);
  }
  dom.window.close();
});
test('unpublished catalog never exposes a download or installer command for any selection', async () => {
  const v = await mount();
  for (const os of ['windows', 'macos', 'linux']) for (const arch of ['x86_64', 'aarch64']) {
    selectPlatform(v, os, arch);
    assert.equal(v.$('#download-link').hidden, true);
    assert.equal(v.$('#download-link').hasAttribute('href'), false);
    assert.equal(v.$('#download-unavailable').disabled, true);
    assert.equal(v.$('#install-command').hidden, true);
    assert.match(v.$('#download-reason').textContent, /No verified public installer/);
  }
  assert.match(v.$('#service-status').textContent, /unknown/);
  v.dom.window.close();
});
test('verified catalog drives downloads, commands, pack filtering, and release notes', async () => {
  let copied;
  const v = await mount({ catalog: fixture, clipboard: { writeText: async value => { copied = value; } } });
  selectPlatform(v);
  assert.equal(v.$('#download-link').hidden, false);
  assert.equal(v.$('#download-link').href, fixture.downloads[0].url);
  assert.equal(v.$('#install-command').hidden, false);
  v.$('#copy-command').click();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(copied, "curl -fsSL 'https://downloads.example.test/install.sh' | bash");
  assert.match(v.$('#copy-status').textContent, /copied/);
  assert.equal(v.all('.pack-card').length, 2);
  assert.equal(v.$('.pack-card img'), null, 'catalog text must not execute as HTML');
  v.$('[data-filter="stuff"]').click();
  assert.equal(v.all('.pack-card').length, 1);
  assert.equal(v.$('[data-filter="stuff"]').getAttribute('aria-pressed'), 'true');
  v.$('#pack-search').value = 'no match';
  v.$('#pack-search').dispatchEvent(new v.dom.window.Event('input'));
  assert.match(v.$('#pack-results').textContent, /No packs match/);
  assert.match(v.$('#release-notes').textContent, /Fixture release note/);
  selectPlatform(v, 'macos');
  assert.equal(v.$('#download-link').hasAttribute('href'), false, 'old URL removed on unsupported selection');
  assert.equal(v.$('#command-text').textContent, '');
  v.dom.window.close();
});
test('catalog failure and invalid URLs fail closed', async () => {
  for (const config of [
    { fail: true },
    { catalog: { ...fixture, schema: 2 } },
    { catalog: { ...fixture, downloads: [{ ...fixture.downloads[0], url: 'javascript:alert(1)' }] } },
    { catalog: { ...fixture, downloads: [{ ...fixture.downloads[0], install_script: 'https://host.test/$(bad).sh' }] } },
    { catalog: { ...fixture, downloads: [...fixture.downloads, ...fixture.downloads] } },
    { catalog: { ...fixture, published_at: null } },
  ]) {
    const v = await mount(config);
    selectPlatform(v);
    assert.equal(v.$('#download-link').hasAttribute('href'), false);
    assert.match(v.$('#download-state').textContent, /unavailable/);
    assert.match(v.$('#pack-results').textContent, /could not be loaded/);
    v.dom.window.close();
  }
});
test('browser platform is a suggestion; architecture remains unanswered', async () => {
  const v = await mount({ platform: 'Linux x86_64' });
  assert.equal(v.$('[data-os="linux"]').getAttribute('aria-pressed'), 'true');
  assert.equal(v.$('#architecture').value, '');
  assert.match(v.$('#platform-hint').textContent, /suggested/);
  v.dom.window.close();
});
test('health is separate, timestamped, and stale checks cannot claim operational status', async () => {
  for (const [age, expected] of [[0, /Services operational/], [3600000, /Status unknown.*stale/]]) {
    const v = await mount({ health: { schema: 1, status: 'operational', checked_at: new Date(Date.now() - age).toISOString() } });
    assert.match(v.$('#service-status').textContent, expected);
    assert.match(v.$('#service-status').textContent, /Checked/);
    v.dom.window.close();
  }
});
test('clipboard failure gives a manual alternative', async () => {
  const v = await mount({ catalog: fixture, clipboard: { writeText: async () => { throw new Error('denied'); } } });
  selectPlatform(v);
  v.$('#copy-command').click();
  await new Promise(resolve => setImmediate(resolve));
  assert.match(v.$('#copy-status').textContent, /copy it manually/);
  v.dom.window.close();
});

test('browser region selects language automatically without a selector or saved override', async () => {
  for (const [languages, expected] of [
    [['pt-BR', 'en-US'], 'pt-BR'], [['en-US', 'pt-BR'], 'en'],
    [['pt-PT'], 'pt-BR'], [['fr-FR', 'pt-BR'], 'pt-BR'], [['fr-FR'], 'en'], [[], 'en'],
  ]) {
    const v = await mount({ languages, savedLanguage: expected === 'en' ? 'pt-BR' : 'en' });
    assert.equal(v.doc.documentElement.lang, expected);
    assert.equal(v.$('#site-language'), null);
    assert.equal(v.$('.language-picker'), null);
    assert.match(v.$('h1').textContent, expected === 'en' ? /More life/ : /Mais vida/);
    assert.equal(v.$('.header > .button').getAttribute('href'), '#download');
    v.dom.window.close();
  }
});
test('automatic Portuguese translates metadata, accessibility labels and empty states without storage', async () => {
  const v = await mount({ languages: ['pt-BR'], blockedStorage: true });
  assert.equal(v.doc.documentElement.lang, 'pt-BR');
  assert.match(v.doc.title, /Mais vida/);
  assert.equal(v.$('#pack-search').getAttribute('aria-label'), 'Buscar pacotes publicados');
  assert.match(v.$('#pack-results').textContent, /Uma nova coleção vem aí/);
  assert.match(v.$('#release-notes').textContent, /O primeiro capítulo/);
  selectPlatform(v);
  assert.match(v.$('#download-reason').textContent, /Ainda não há um instalador/);
  v.dom.window.close();
});
test('Portuguese handles errors, copy feedback, localized catalog content, dates and sizes', async () => {
  const catalog = structuredClone(fixture);
  catalog.packs[0].translations = { 'pt-BR': { name: 'Expansão de teste' } };
  catalog.releases[0].translations = { 'pt-BR': { notes: ['Novidade de teste.'] } };
  catalog.packs[0].download_size = 1610612736;
  const v = await mount({ catalog, languages: ['pt-BR'], clipboard: { writeText: async () => {} }, health: { schema: 1, status: 'operational', checked_at: new Date().toISOString() } });
  selectPlatform(v);
  assert.match(v.$('#pack-results').textContent, /Expansão de teste/);
  assert.match(v.$('#pack-results').textContent, /1,5 GB/);
  assert.match(v.$('#release-notes').textContent, /Novidade de teste/);
  assert.match(v.$('#release-notes').textContent, /nov/);
  assert.match(v.$('#service-status').textContent, /Serviços funcionando.*Verificado em/);
  v.$('#copy-command').click();
  await new Promise(resolve => setImmediate(resolve));
  assert.match(v.$('#copy-status').textContent, /Comando de instalação copiado/);
  v.$('#pack-search').value = 'expansão';
  v.$('#pack-search').dispatchEvent(new v.dom.window.Event('input'));
  assert.equal(v.all('.pack-card').length, 1);
  v.dom.window.close();
  const failed = await mount({ fail: true, languages: ['pt-BR'] });
  assert.match(failed.$('#download-state').textContent, /indisponíveis/);
  assert.match(failed.$('#pack-results').textContent, /Não foi possível carregar/);
  assert.match(failed.$('#release-notes').textContent, /indisponíveis/);
  failed.dom.window.close();
});
