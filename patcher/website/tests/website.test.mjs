import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { JSDOM } from 'jsdom';

const root = new URL('../', import.meta.url);
const html = await readFile(new URL('index.html', root), 'utf8');
const translations = (await readFile(new URL('i18n.js', root), 'utf8')).replace(/export /g, '');
const source = translations + '\n' + (await readFile(new URL('app.js', root), 'utf8')).replace(/^import .*?;\n/, '').replace(/export function /g, 'function ');
const packReference = JSON.parse(await readFile(new URL('data/packs.json', root), 'utf8'));
const empty = JSON.parse(await readFile(new URL('data/catalog.json', root), 'utf8'));
const fixture = {
  schema: 1, published_at: '2026-11-09T12:00:00Z',
  downloads: [{ os: 'linux', arch: 'x86_64', offered: true, version: 'test-1', url: 'https://downloads.example.test/linux.tar.gz', install_script: 'https://downloads.example.test/install.sh' }],
  packs: [
    { id: 'test-expansion', name: 'Fixture expansion', kind: 'expansion', platforms: ['linux'], download_size: 1073741824 },
    { id: 'test-kit', name: '<img src=x onerror=alert(1)>', kind: 'kit', platforms: ['linux'], download_size: 4096 },
  ],
  releases: [{ version: 'test-1', date: '2026-11-09', notes: ['Fixture release note.'] }],
};
async function mount({ catalog = empty, health = { schema: 1, status: 'unknown', checked_at: null }, fail = false, platform = '', hints, clipboard, languages = ['en-US'], savedLanguage, blockedStorage = false } = {}) {
  const dom = new JSDOM(html, { url: 'https://website.example.test/', runScripts: 'outside-only' });
  Object.defineProperty(dom.window.navigator, 'platform', { value: platform });
  if (hints) Object.defineProperty(dom.window.navigator, 'userAgentData', { value: { platform, getHighEntropyValues: async () => hints } });
  Object.defineProperty(dom.window.navigator, 'languages', { value: languages });
  Object.defineProperty(dom.window.navigator, 'language', { value: languages[0] });
  if (savedLanguage) dom.window.localStorage.setItem('patcher-website-language', savedLanguage);
  if (blockedStorage) Object.defineProperty(dom.window, 'localStorage', { get() { throw new Error('blocked'); } });
  if (clipboard) Object.defineProperty(dom.window.navigator, 'clipboard', { value: clipboard });
  dom.window.fetch = async (path) => {
    if (fail) throw new Error('offline');
    return { ok: true, json: async () => structuredClone(path.includes('catalog') ? catalog : path.includes('packs.json') ? packReference : health) };
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
  for (const [os, arch] of [['windows', 'x86_64'], ['linux', 'x86_64'], ['macos', 'x86_64'], ['macos', 'aarch64']]) {
    selectPlatform(v, os, arch);
    assert.equal(v.$('#download-link').hidden, true);
    assert.equal(v.$('#download-link').hasAttribute('href'), false);
    assert.equal(v.$('#download-unavailable').disabled, true);
    assert.equal(v.$('#install-command').hidden, true);
    assert.match(v.$('#download-reason').textContent, /No installer is offered for this system yet/);
  }
  assert.match(v.$('#service-status').textContent, /unknown/);
  v.dom.window.close();
});
test('offered catalog drives downloads, commands, and release notes; pack listings', async () => {
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
  assert.match(v.$('#download-reason').textContent, /Available for Linux \(x64\)/);
  assert.equal(v.$('img[src="x"]'), null, 'catalog text must not execute as HTML');
  assert.match(v.$('#pack-list').textContent, /Fixture expansion/);
  v.$('#pack-search').value = 'test-kit';
  v.$('#pack-search').dispatchEvent(new v.dom.window.Event('input'));
  assert.doesNotMatch(v.$('#pack-list').textContent, /Fixture expansion/);
  assert.match(v.$('#pack-count').textContent, /1 of 2/);
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
    // The game does not run on Linux or Windows on ARM, so a catalog listing either is rejected.
    { catalog: { ...fixture, downloads: [{ ...fixture.downloads[0], arch: 'aarch64' }] } },
    { catalog: { ...fixture, downloads: [{ ...fixture.downloads[0], os: 'windows', arch: 'aarch64', install_script: 'https://downloads.example.test/install.ps1' }] } },
    { catalog: { ...fixture, published_at: null } },
  ]) {
    const v = await mount(config);
    selectPlatform(v);
    assert.equal(v.$('#download-link').hasAttribute('href'), false);
    assert.match(v.$('#download-state').textContent, /unavailable/);
    assert.match(v.$('#release-notes').textContent, /unavailable/);
    v.dom.window.close();
  }
});
test('Windows and Linux need no architecture: a detected Linux shows its download at once', async () => {
  const v = await mount({ catalog: fixture, platform: 'Linux x86_64' });
  assert.equal(v.$('[data-os="linux"]').getAttribute('aria-pressed'), 'true');
  assert.equal(v.$('.architecture').hidden, true);
  assert.equal(v.$('#download-link').hidden, false);
  assert.equal(v.$('#install-command').hidden, false);
  assert.match(v.$('#platform-hint').textContent, /Linux detected/);
  assert.match(v.$('#download-state').textContent, /Linux · x64/);
  v.$('[data-os="windows"]').click();
  assert.equal(v.$('.architecture').hidden, true);
  assert.equal(v.$('#unavailable-label').textContent, 'Not available for this system');
  v.dom.window.close();
});
test('only a Mac is asked for its processor, and a reported one is preselected', async () => {
  const asked = await mount({ catalog: fixture, platform: 'MacIntel' });
  assert.equal(asked.$('.architecture').hidden, false);
  assert.equal(asked.$('#architecture').value, '');
  assert.equal(asked.$('#unavailable-label').textContent, 'Choose your Mac');
  assert.match(asked.$('#platform-hint').textContent, /Choose Intel or Apple silicon/);
  asked.dom.window.close();
  const reported = await mount({ catalog: fixture, platform: 'macOS', hints: { architecture: 'arm', bitness: '64' } });
  assert.equal(reported.$('#architecture').value, 'aarch64');
  assert.match(reported.$('#platform-hint').textContent, /macOS · Apple silicon detected/);
  assert.equal(reported.$('#unavailable-label').textContent, 'Not available for this system');
  reported.dom.window.close();
  const none = await mount({ catalog: fixture });
  assert.equal(none.$('#unavailable-label').textContent, 'Choose your system first');
  none.dom.window.close();
});
test('a 32-bit or unknown report leaves the Mac choice to the visitor', async () => {
  for (const hints of [{ architecture: 'x86', bitness: '32' }, { architecture: 'riscv', bitness: '64' }]) {
    const v = await mount({ catalog: fixture, platform: 'macOS', hints });
    assert.equal(v.$('#architecture').value, '');
    v.dom.window.close();
  }
});
test('health names the download service, is timestamped, and stale checks claim nothing', async () => {
  for (const [age, expected] of [[0, /^Download service reachable/], [3600000, /Download service status unknown.*stale/]]) {
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
    assert.match(v.$('h1').textContent, expected === 'en' ? /See your Sims 4 packs/ : /Veja seus pacotes/);
    assert.equal(v.$('.header > .button').getAttribute('href'), '#download');
    v.dom.window.close();
  }
});
test('automatic Portuguese translates metadata, accessibility labels and empty states without storage', async () => {
  const v = await mount({ languages: ['pt-BR'], blockedStorage: true });
  assert.equal(v.doc.documentElement.lang, 'pt-BR');
  assert.match(v.doc.title, /Universal Patcher para The Sims 4/);
  assert.equal(v.$('#copy-command').getAttribute('aria-label'), 'Copiar comando de instalação');
  assert.match(v.$('#release-notes').textContent, /Ainda não há novidades/);
  selectPlatform(v);
  assert.match(v.$('#download-reason').textContent, /Ainda não há instalador oferecido/);
  v.dom.window.close();
});
test('Portuguese handles errors, copy feedback, localized release notes and dates', async () => {
  const catalog = structuredClone(fixture);
  catalog.releases[0].translations = { 'pt-BR': { notes: ['Novidade de teste.'] } };
  const v = await mount({ catalog, languages: ['pt-BR'], clipboard: { writeText: async () => {} }, health: { schema: 1, status: 'operational', checked_at: new Date().toISOString() } });
  selectPlatform(v);
  assert.match(v.$('#download-reason').textContent, /Disponível para Linux \(x64\)/);
  assert.match(v.$('#release-notes').textContent, /Novidade de teste/);
  assert.match(v.$('#release-notes').textContent, /nov/);
  assert.match(v.$('#service-status').textContent, /Serviço de download acessível.*Verificado em/);
  v.$('#copy-command').click();
  await new Promise(resolve => setImmediate(resolve));
  assert.match(v.$('#copy-status').textContent, /Comando de instalação copiado/);
  v.dom.window.close();
  const failed = await mount({ fail: true, languages: ['pt-BR'] });
  assert.match(failed.$('#download-state').textContent, /indisponíveis/);
  assert.match(failed.$('#release-notes').textContent, /indisponíveis/);
  failed.dom.window.close();
});

const tokens = JSON.parse(await readFile(new URL('../design/tokens.json', root), 'utf8'));
const css = await readFile(new URL('styles.css', root), 'utf8');
const cssVars = Object.fromEntries([...css.match(/:root\s*\{[^}]*\}/)[0].matchAll(/--([\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name, value.trim()]));
function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255).map(c => c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

test('styles.css custom properties mirror design/tokens.json', () => {
  for (const [name, value] of Object.entries(tokens.color)) assert.equal(cssVars[name.replace(/_/g, '-')]?.toLowerCase(), value, name);
  assert.equal(cssVars.font, tokens.font.family);
  tokens.space.forEach((value, index) => assert.equal(cssVars[`space-${index + 1}`], `${value}px`));
  for (const [name, value] of Object.entries(tokens.radius)) assert.equal(cssVars[`radius-${name}`], `${value}px`);
  for (const [name, value] of Object.entries(tokens.motion)) assert.equal(cssVars[`motion-${name.replace(/_ms$/, '')}`], `${value}ms`);
  // Every colour in the stylesheet comes from a token; no second font family is declared.
  const outsideRoot = css.replace(/:root\s*\{[^}]*\}/, '');
  assert.deepEqual(outsideRoot.match(/#[0-9a-f]{3,8}\b/gi) ?? [], []);
  assert.doesNotMatch(css, /Georgia|\bInter\b|[^-]serif\b|italic/);
});
test('every text and background pair the stylesheet uses meets WCAG AA (4.5:1)', () => {
  const pairs = [
    ['ink', 'paper'], ['ink', 'surface'], ['ink', 'mint'], ['ink', 'line'],
    ['muted', 'paper'], ['muted', 'surface'],
    ['teal', 'paper'], ['teal', 'surface'], ['teal', 'mint'],
    ['on-teal', 'teal'], ['on-teal', 'teal-hover'],
    ['paper', 'ink'], ['mint', 'ink'], ['warning', 'paper'], ['error', 'paper'],
  ];
  for (const [text, background] of pairs) {
    const ratio = contrast(cssVars[text], cssVars[background]);
    assert.ok(ratio >= 4.5, `${text} on ${background} is ${ratio.toFixed(2)}:1`);
  }
});
test('the page makes no claim the app cannot back', async () => {
  const banned = [/up to date/i, /verified release/i, /verified public installer/i, /operational/i, /\bpause\b/i, /resume/i, /maintenance/i, /keeps itself/i,
    /Interface concept/i, /EST\. FOR/i, /worth looking after/i, /Good things take care/i, /Small changes/i, /Make yourself at home/i, /only after verification/i];
  const healthy = { schema: 1, status: 'operational', checked_at: new Date().toISOString() };
  for (const config of [{}, { catalog: fixture, health: healthy }, { fail: true }]) {
    const v = await mount(config);
    for (const [os, arch] of [['windows', 'x86_64'], ['linux', 'x86_64'], ['macos', 'aarch64']]) {
      selectPlatform(v, os, arch);
      v.all('[data-catalog-copy]').forEach(element => element.remove());
      const text = `${v.doc.title} ${v.$('meta[name="description"]').content} ${v.doc.body.textContent}`;
      for (const phrase of banned) assert.doesNotMatch(text, phrase);
    }
    v.dom.window.close();
  }
  const pt = await mount({ catalog: fixture, health: healthy, languages: ['pt-BR'] });
  selectPlatform(pt);
  for (const phrase of [/atualizar o The Sims 4/i, /pausar/i, /manutenção/i, /versão verificada/i, /funcionando/i]) assert.doesNotMatch(pt.doc.body.textContent, phrase);
  pt.dom.window.close();
});
test('catalogs that still say "verified" render downloads; contradictory ones are refused', async () => {
  const old = structuredClone(fixture);
  delete old.downloads[0].offered;
  old.downloads[0].verified = true;
  const v = await mount({ catalog: old });
  selectPlatform(v);
  assert.equal(v.$('#download-link').hidden, false);
  assert.equal(v.$('#download-link').href, fixture.downloads[0].url);
  v.dom.window.close();
  const hidden = structuredClone(old);
  hidden.downloads[0].verified = false;
  const h = await mount({ catalog: hidden });
  selectPlatform(h);
  assert.equal(h.$('#download-link').hidden, true);
  assert.match(h.$('#download-reason').textContent, /No installer is offered/);
  h.dom.window.close();
  for (const downloads of [[{ ...fixture.downloads[0], verified: false }], [{ ...old.downloads[0], verified: undefined }]]) {
    const bad = await mount({ catalog: { ...fixture, downloads } });
    selectPlatform(bad);
    assert.equal(bad.$('#download-link').hasAttribute('href'), false);
    assert.match(bad.$('#download-state').textContent, /unavailable/);
    bad.dom.window.close();
  }
});
test('a Windows download says the installer is unsigned', async () => {
  const catalog = structuredClone(fixture);
  catalog.downloads = [{ os: 'windows', arch: 'x86_64', offered: true, version: 'test-1', url: 'https://downloads.example.test/setup.exe', install_script: 'https://downloads.example.test/install.ps1' }];
  const v = await mount({ catalog });
  selectPlatform(v, 'windows');
  assert.match(v.$('#download-reason').textContent, /Available for Windows \(x64\)\. Windows may warn .* not code-signed/);
  assert.match(v.$('.availability').textContent, /Available for Windows \(x64\)/);
  v.dom.window.close();
});
test('the 0.1.0 note about updating itself is corrected beside it', async () => {
  const catalog = structuredClone(fixture);
  catalog.releases.push({ version: '0.1.0', date: '2026-11-08', notes: ['Installs for your user with one command and no administrator rights, and keeps itself up to date.'] });
  for (const [languages, expected] of [[['en-US'], /Correction: the patcher does not update on its own/], [['pt-BR'], /Correção: o patcher não se atualiza sozinho/]]) {
    const v = await mount({ catalog, languages });
    const article = v.all('#release-notes article').find(item => /0\.1\.0/.test(item.textContent));
    assert.match(article.querySelector('.correction').textContent, expected);
    assert.equal(v.all('.correction').length, 1);
    v.dom.window.close();
  }
});
test('every piece of page copy has a Portuguese translation', async () => {
  const texts = doc => {
    const walker = doc.createTreeWalker(doc.body, 4);
    const out = [];
    let item;
    while ((item = walker.nextNode())) if (item.textContent.trim() && !item.parentElement.closest('script,style,code,noscript,[data-catalog-copy]')) out.push(item.textContent.trim());
    return out;
  };
  const en = await mount();
  const pt = await mount({ languages: ['pt-BR'] });
  const same = new Set(['universal', 'patcher', 'Windows', 'macOS', 'Linux', 'Intel', 'Apple silicon', '+', 'Download', '01', '02', '03', '04']);
  const [english, portuguese] = [texts(en.doc), texts(pt.doc)];
  assert.equal(english.length, portuguese.length);
  english.forEach((text, index) => { if (!same.has(text)) assert.notEqual(portuguese[index], text, `untranslated: ${text}`); });
  en.dom.window.close();
  pt.dom.window.close();
});

test('reference packs match every named and unnamed engine entry', async () => {
 const rust = await readFile(new URL('../core/src/local.rs', root), 'utf8');
 const section = rust.split('const CATALOG:')[1].split('const PACK_FILES:')[0];
 const ids = [...section.matchAll(/"((?:EP|GP|SP|FP)\d+)"/g)].map(m => m[1]);
 assert.deepEqual(packReference.packs.map(p => p.id), ids);
 const v = await mount();
 assert.equal(v.all('.pack-card').length, ids.length);
 v.dom.window.close();
});
