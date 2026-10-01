/* bake.test.mjs — the Door's /assets bake (deploy-door.sh stage) against the webapp it ships:
   every "assets/…" string literal webapp.js names (the Face editor's scripts and stylesheets,
   which it loads when first wanted) is staged, from the website commit the deploy names, as
   index.html's own are; the editor's stickers, which face.js builds from face-assets.js's
   names at run time, are staged exactly when face.js is; and every file of the webapp's own
   that index.html (src, href) or webapp.js (a quoted relative path) names is staged too
   (NC-56: only what the pages load, and all of it). Nothing is sent.

   DOOR_WEBAPP: the webapp to stage (default app/web/webapp). DOOR_ASSETS_COMMIT: the website
   commit (default the website checkout's HEAD, as the deploy's).

   Run: node --test app/door/hosting/bake.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBAPP = process.env.DOOR_WEBAPP || join(HERE, '..', '..', 'web', 'webapp');

/* The quoted "assets/…" literals of a script, its comments aside. */
function literals(src) {
  const code = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  return [...new Set([...code.matchAll(/['"](assets\/[^'"?#]+)['"]/g)].map((m) => m[1]))];
}

/* The webapp's own files index.html and webapp.js name: relative, not under assets/. */
function own(webapp) {
  const page = readFileSync(join(webapp, 'index.html'), 'utf8');
  const js = readFileSync(join(webapp, 'webapp.js'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  const named = [...page.matchAll(/(?:src|href)="([^"#?]+)/g)].map((m) => m[1])
    .concat([...js.matchAll(/['"]([A-Za-z0-9_.][A-Za-z0-9_.\/-]*\.(?:m?js|css|svg|png|webp|jpe?g|gif|woff2?|json|html|pdf))['"]/g)].map((m) => m[1]));
  return [...new Set(named.filter((n) => !/^(assets\/|\/|[A-Za-z][A-Za-z0-9+.-]*:)/.test(n)))];
}

const env = (webapp) => ({ ...process.env, DOOR_BOX: 'stage', DOOR_HOST: 'stage', DOOR_KEY: '/dev/null', DOOR_WEBAPP: webapp });

/* The webapp as the deploy would ship it, in a directory `stage` names; removed after `f`. */
function staged(f) {
  const out = execFileSync(join(HERE, 'deploy-door.sh'), ['stage'], { env: env(WEBAPP), encoding: 'utf8' });
  const dir = out.trim().split('\n').pop().replace(/^» /, '');
  assert.ok(existsSync(join(dir, 'index.html')), `stage named ${dir}`);
  try {
    return f(dir);
  } finally {
    rmSync(dirname(dir), { recursive: true, force: true });
  }
}

test('every "assets/…" literal webapp.js names is staged', () => {
  const names = literals(readFileSync(join(WEBAPP, 'webapp.js'), 'utf8'));
  staged((dir) => {
    const missing = names.filter((n) => !existsSync(join(dir, n)));
    assert.deepEqual(missing, [], `webapp.js loads what the bake left out (of ${names.length})`);
  });
});

test("the Face editor's stickers are staged exactly when face.js is", () => {
  staged((dir) => {
    const editor = existsSync(join(dir, 'assets/face/face.js'));
    const assets = readFileSync(join(dir, 'assets/face/vendor/face-assets.js'), 'utf8');
    const stickers = [...assets.matchAll(/"file":\s*"([^"]+)"/g)].map((m) => join('assets/face/vendor', m[1]));
    assert.ok(stickers.length > 0, 'face-assets.js names its stickers');
    const held = stickers.filter((s) => existsSync(join(dir, s)));
    assert.deepEqual(held.length, editor ? stickers.length : 0, `face.js staged: ${editor}; stickers staged: ${held.length} of ${stickers.length}`);
  });
});

test('every file of its own index.html and webapp.js name is staged', () => {
  const names = own(WEBAPP);
  assert.ok(names.includes('webapp.js'), `index.html names webapp.js: ${names}`);
  staged((dir) => {
    const missing = names.filter((n) => !existsSync(join(dir, n)));
    assert.deepEqual(missing, [], `the pages load what the stage left out (of ${names.length})`);
  });
});

/* A copy of the webapp whose index.html also carries `tag`. */
function webappWith(tag) {
  const dir = mkdtempSync(join(process.env.TMPDIR || tmpdir(), 'webapp-'));
  cpSync(WEBAPP, dir, { recursive: true });
  const page = join(dir, 'index.html');
  writeFileSync(page, readFileSync(page, 'utf8').replace('</body>', `${tag}\n</body>`));
  return dir;
}

test('a page naming a file the webapp does not hold, or one outside it, stops the stage, named', () => {
  for (const [tag, why] of [
    ['<script src="nowhere.js"></script>', 'nowhere.js (named by index.html): not in the webapp'],
    ['<script src="../outside.js"></script>', '../outside.js (named by index.html): outside the webapp'],
  ]) {
    const dir = webappWith(tag);
    try {
      const r = spawnSync(join(HERE, 'deploy-door.sh'), ['stage'], { env: env(dir), encoding: 'utf8' });
      assert.notEqual(r.status, 0, `${tag}: the stage goes on`);
      assert.ok(r.stderr.includes(why), `${tag}: ${r.stderr.trim()}`);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
});
