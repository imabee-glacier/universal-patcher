import { chromium } from 'playwright';
import assert from 'node:assert/strict';
const browser=await chromium.launch();
const page=await browser.newPage({ignoreHTTPSErrors:true});
const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.goto(process.argv[2],{waitUntil:'networkidle'});
assert.equal(await page.locator('.pack-card').count(),106);
await page.locator('#pack-search').fill('EP01');
assert.equal(await page.locator('.pack-card').count(),1);
assert.match(await page.locator('#pack-list').innerText(),/Get to Work/);
await page.locator('#pack-search').fill('');
assert.equal(await page.locator('#platform-list p').count(),4);
assert.ok(await page.locator('#platform-list a').count()>0);
assert.ok(await page.locator('#release-notes article').count()>0);
for(const width of [1280,720,390]) {
 await page.setViewportSize({width,height:800});
 await page.locator('#packs').scrollIntoViewIfNeeded();
 assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
 await page.screenshot({path:`/shots/packs-${width}.png`});
}
assert.deepEqual(errors,[]);await browser.close();console.log('106 packs, search, platforms, release notes and three viewport widths passed');
