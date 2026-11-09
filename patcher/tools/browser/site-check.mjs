// Loads the published website in Chromium and checks what a visitor sees: no Content Security
// Policy violations or script errors, a verified Linux download with its one-line command,
// release notes and a fresh service status from the published data, and no download offered
// for a platform that has not been verified.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';

const [origin, shots] = process.argv.slice(2);
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: process.env.TEST_CA === '1', viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const problems = [];
page.on('console', m => { if (m.type() === 'error') problems.push(`console: ${m.text()}`); });
page.on('pageerror', e => problems.push(`page error: ${e.message}`));
await page.addInitScript(() => {
  document.addEventListener('securitypolicyviolation', e => console.error(`CSP violation: ${e.violatedDirective} ${e.blockedURI}`));
});

const response = await page.goto(origin, { waitUntil: 'networkidle' });
assert.equal(response.status(), 200);
assert.match(response.headers()['content-security-policy'] ?? '', /require-trusted-types-for 'script'/);

await page.click('[data-os="linux"]');
await page.selectOption('#architecture', 'x86_64');
await page.locator('#download-link').waitFor({ state: 'visible' });
assert.equal(await page.textContent('#command-text'), `curl -fsSL '${origin}install.sh' | bash`);
const catalog = await (await page.request.get(`${origin}data/catalog.json`)).json();
const latest = catalog.releases.at(-1);
const notes = await page.textContent('#release-notes');
assert.ok(notes.includes(`Version ${latest.version}`) && latest.notes.every(n => notes.includes(n)), `release notes for ${latest.version} are not shown`);
assert.match(await page.textContent('#service-status'), /Services operational/);
await page.screenshot({ path: `${shots}/site-linux.png`, fullPage: true });

await page.click('[data-os="windows"]');
await page.selectOption('#architecture', 'x86_64');
assert.equal(await page.locator('#download-link').isVisible(), false, 'unverified platforms offer no download');
assert.equal(await page.locator('#install-command').isVisible(), false);

await page.setViewportSize({ width: 720, height: 560 });
await page.screenshot({ path: `${shots}/site-720x560.png` });
const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
assert.ok(overflow <= 0, `page scrolls sideways by ${overflow}px at 720 wide`);

assert.deepEqual(problems, []);
await browser.close();
console.log('website checks passed');
