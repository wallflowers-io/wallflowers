#!/usr/bin/env node
// Drive one of these pages in a real browser and print what it said.
//
// WHY A DRIVER AND NOT A NODE PORT. `core-wasm.mjs` and `relay-pair.mjs` load the
// same module under Node, which proves the module. It does not prove the PAGE:
// `fetch` of a wasm MIME type, `crypto.subtle` on an insecure origin, an import
// the page forgot to supply — every one of those is a browser-only failure, and
// every one of them is how a wasm page has actually broken here. So this loads
// the page over http, in chromium, and reports the element the page writes into.
//
//   node smoke/page.mjs <url> [--sel '#o'] [--until 'all green|FAILED'] [--ms 30000]
//
// Exit code is 1 if the page printed FAIL/ERROR/REJECTED, or if the sentinel
// never appeared — a page that sits on "loading…" is a failure, not a pass.
import { chromium } from 'playwright-core';

const args = process.argv.slice(2);
const url = args[0];
if (!url) { console.error('usage: node smoke/page.mjs <url> [--sel sel] [--until re] [--ms n]'); process.exit(2); }
const opt = (n, d) => { const i = args.indexOf('--' + n); return i < 0 ? d : args[i + 1]; };
const sel = opt('sel', '#o');
const until = new RegExp(opt('until', 'all green|FAILED|could not'), 'i');
const ms = Number(opt('ms', '30000'));

const EXE = process.env.CHROME_HEADLESS_SHELL ||
  process.env.HOME + '/Library/Caches/ms-playwright/chromium_headless_shell-1243/' +
  'chrome-headless-shell-mac-arm64/chrome-headless-shell';

const browser = await chromium.launch({ executablePath: EXE, args: ['--no-sandbox'] });
const page = await browser.newPage();
const console_ = [];
page.on('console', (m) => console_.push('  [' + m.type() + '] ' + m.text()));
page.on('pageerror', (e) => console_.push('  [pageerror] ' + e.message));
page.on('requestfailed', (r) => console_.push('  [requestfailed] ' + r.url() + ' ' + (r.failure() || {}).errorText));

let timedOut = false;
try {
  await page.goto(url, { waitUntil: 'load', timeout: ms });
  await page.waitForFunction(
    ([s, re]) => {
      const el = document.querySelector(s);
      return !!el && new RegExp(re, 'i').test(el.textContent || '');
    },
    [sel, until.source],
    { timeout: ms, polling: 250 }
  );
} catch (e) {
  timedOut = true;
  console_.push('  [driver] ' + String(e.message).split('\n')[0]);
}
const body = await page.evaluate((s) => {
  const el = document.querySelector(s);
  return el ? el.textContent : '(no ' + s + ' on the page)';
}, sel);
await browser.close();

console.log(body);
if (console_.length) console.log('\nbrowser console:\n' + console_.join('\n'));
// ANCHORED, not a substring search. These pages narrate — one of them says the
// word "REJECTED" inside a line that is passing — so the failure marks are the
// ones the pages actually emit at the START of a line, plus the summary.
const MARKS = [/(^|\n)\s*FAIL\b/, /(^|\n)ERROR\b/, /(^|\n)REJECTED\b/,
               /\b\d+ FAILED\b/, /could not instantiate/i];
const bad = timedOut || MARKS.some((re) => re.test(body));
console.log('\n' + (bad ? 'PAGE FAILED — ' + url : 'page green — ' + url));
process.exit(bad ? 1 : 0);
