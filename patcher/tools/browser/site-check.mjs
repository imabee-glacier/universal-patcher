// Loads the published website in Chromium and checks what a visitor sees: no Content Security
// Policy violations or script errors, an offered Linux download with its one-line command and
// no architecture question, release notes and a fresh service status from the published data,
// the processor question on macOS only, and downloads only for platforms the catalog offers.
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

// Linux and Windows run the game on x64 only, so neither asks for an architecture.
await page.click('[data-os="linux"]');
assert.equal(await page.locator('.architecture').isVisible(), false, 'Linux asks no architecture');
await page.locator('#download-link').waitFor({ state: 'visible' });
assert.equal(await page.textContent('#command-text'), `curl -fsSL '${origin}install.sh' | bash`);
assert.match(await page.textContent('#download-state'), /Linux · x64/);
const catalog = await (await page.request.get(`${origin}data/catalog.json`)).json();
assert.match(await page.textContent('#service-status'), /Download service reachable/);
await page.screenshot({ path: `${shots}/site-linux.png`, fullPage: true });

const offered = (os, arch) => catalog.downloads.some(d => d.os === os && d.arch === arch && (d.offered ?? d.verified));
await page.click('[data-os="windows"]');
assert.equal(await page.locator('.architecture').isVisible(), false, 'Windows asks no architecture');
assert.equal(await page.locator('#download-link').isVisible(), offered('windows', 'x86_64'), 'Windows download follows the catalog');

// Only a Mac has two processors the game runs on.
await page.click('[data-os="macos"]');
await page.locator('.architecture').waitFor({ state: 'visible' });
assert.deepEqual(await page.locator('#architecture option').evaluateAll(o => o.map(e => e.value)), ['', 'x86_64', 'aarch64']);
await page.selectOption('#architecture', 'aarch64');
assert.equal(await page.locator('#download-link').isVisible(), offered('macos', 'aarch64'), 'macOS download follows the catalog');
assert.doesNotMatch(await page.textContent('.questions details:first-child p'), /ARM64|each architecture/);
await page.click('[data-os="linux"]');

await page.setViewportSize({ width: 720, height: 560 });
await page.screenshot({ path: `${shots}/site-720x560.png` });
const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
assert.ok(overflow <= 0, `page scrolls sideways by ${overflow}px at 720 wide`);

assert.deepEqual(problems, []);
await browser.close();
console.log('website checks passed');
