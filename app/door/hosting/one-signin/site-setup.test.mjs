import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path'; import http from 'node:http'; import crypto from 'node:crypto';
import zlib from 'node:zlib'; import assert from 'node:assert/strict'; import { spawn, execFileSync } from 'node:child_process'; import { fileURLToPath } from 'node:url';
// site-setup.mjs (W-90) against a stub Door on loopback that holds every call to the real
// Door's DPoP rules (token.rs: the proof's key, signature, htm and htu, iat, a fresh jti, ath),
// and a stand-in for the Arc's bundles. The real Door's half is security.rs's w90_ tests.
const CLI = fileURLToPath(new URL('./site-setup.mjs', import.meta.url));
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HOST = 'h'.repeat(64), HR = 'e'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
const [R, S, F] = ['r', 's', 'f'].map(c => c.repeat(64));
const CLIENT = 'wallflowers-setup-egregore';
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'site-setup-test-'));
process.on('exit', () => fs.rmSync(tmp, { recursive: true, force: true }));
const b64u = b => Buffer.from(b).toString('base64url');
const sha = s => crypto.createHash('sha256').update(s).digest();

// A PNG, drawn here: w by h, RGB (2) or RGBA (6), each pixel from px(x, y).
function png(file, w, h, type, px) {
  const n = type === 6 ? 4 : 3, raw = Buffer.alloc((w * n + 1) * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) px(x, y).forEach((v, i) => { raw[y * (w * n + 1) + 1 + x * n + i] = v; });
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const c = Buffer.alloc(4); c.writeUInt32BE(zlib.crc32(Buffer.concat([Buffer.from(t), d]))); return Buffer.concat([l, Buffer.from(t), d, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = type;
  fs.writeFileSync(file, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]));
  return file;
}
const TINY = png(path.join(tmp, 'tiny.png'), 2, 2, 6, () => [200, 40, 90, 255]);
// A photograph's worth: a gradient under fine noise, far over 156,000 base64 as a PNG.
let seed = 7; const noise = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) % 12;
const BIG = png(path.join(tmp, 'big.png'), 1800, 1200, 2, (x, y) => [(x / 7 + noise()) & 255, (y / 5 + noise()) & 255, ((x + y) / 12 + noise()) & 255]);
assert.ok(fs.statSync(BIG).size * 4 / 3 > 156000, 'the big mark is over the limit before it is drawn smaller');
// A picture with alpha too big as a PNG even at 384 px: the snippet then asks for a JPEG (15e6eb43).
const BIG_ALPHA = png(path.join(tmp, 'big-alpha.png'), 1800, 1200, 6, (x, y) => [(x / 7 + noise()) & 255, (y / 5 + noise()) & 255, ((x + y) / 12 + noise()) & 255, 255 - noise() * 8]);
execFileSync('sips', ['-Z', '384', BIG_ALPHA, '--out', path.join(tmp, 'big-alpha-384.png')], { stdio: 'ignore' });
assert.ok(fs.statSync(path.join(tmp, 'big-alpha-384.png')).size * 4 / 3 > 156000, 'the alpha mark is over the limit as a PNG at 384 px');

const listen = (s, port = 0) => new Promise(res => s.listen(port, '127.0.0.1', () => res(s.address().port)));
const freePort = async () => { const s = http.createServer(); const p = await listen(s); await new Promise(r => s.close(r)); return p; };

// The Arc's bundles: distinct each time; counted.
let bundlesServed = 0;
const arc = http.createServer((q, r) => { r.end('bundle-' + (++bundlesServed)); });
const ARC_URL = `http://127.0.0.1:${await listen(arc)}/v1/bundle`;

// THE STUB DOOR. `world` is the account as a Door session reads it; `calls` what reached it.
let world, calls, base, codeFor = null;
const tokens = new Map(), jtis = new Set(), issued = [];
function proven(q, route, token) {
  const [h, p, s] = String(q.headers.dpop || '').split('.');
  const hd = JSON.parse(Buffer.from(h, 'base64url')), cl = JSON.parse(Buffer.from(p, 'base64url'));
  assert.deepEqual([hd.typ, hd.alg, hd.jwk.kty, hd.jwk.crv], ['dpop+jwt', 'ES256', 'EC', 'P-256'], route + ': the proof');
  const key = crypto.createPublicKey({ key: { kty: 'EC', crv: 'P-256', x: hd.jwk.x, y: hd.jwk.y }, format: 'jwk' });
  assert.ok(crypto.verify('sha256', Buffer.from(h + '.' + p), { key, dsaEncoding: 'ieee-p1363' }, Buffer.from(s, 'base64url')), route + ': the proof verifies');
  assert.deepEqual([cl.htm, cl.htu], [q.method, base + route], route + ': htm and htu');
  assert.ok(Math.abs(cl.iat - Date.now() / 1000) <= 60, route + ': iat');
  assert.ok(cl.jti && !jtis.has(cl.jti), route + ': a fresh jti'); jtis.add(cl.jti);
  if (token) assert.equal(cl.ath, b64u(sha(token)), route + ': ath');
  return b64u(sha(JSON.stringify({ crv: 'P-256', kty: 'EC', x: hd.jwk.x, y: hd.jwk.y })));
}
const door = http.createServer(async (q, r) => {
  let body = ''; for await (const c of q) body += c;
  const b = body ? JSON.parse(body) : null, route = new URL(q.url, base).pathname;
  const answer = (code, j) => { r.writeHead(code, { 'content-type': typeof j === 'string' ? 'text/plain' : 'application/json' }); r.end(typeof j === 'string' ? j : JSON.stringify(j)); };
  try {
    if (route === '/v2/token') {
      const jkt = proven(q, route);
      assert.ok(codeFor && b.code === codeFor.code, 'the code the window gave');
      assert.deepEqual([b.client, b.redirect_uri], [CLIENT, codeFor.callback], 'the client and its callback');
      assert.equal(b64u(sha(b.code_verifier)), codeFor.challenge, "the verifier is the window's challenge's");
      codeFor = null;
      const t = crypto.randomBytes(32).toString('hex'); tokens.set(t, jkt); issued.push(t);
      calls.push([route]);
      return answer(200, { access_token: t, token_type: 'DPoP', expires_in: 900, scope: world.scope || SITE });
    }
    const t = (q.headers.authorization || '').replace(/^DPoP /, '');
    assert.ok(tokens.has(t), route + ': the token');
    assert.equal(proven(q, route, t), tokens.get(t), route + ': by the key the token is bound to');
    assert.equal(q.headers.cookie, undefined, route + ': no cookie');
    calls.push([route, b]);
    if (route === '/v2/signout') { tokens.delete(t); return answer(200, {}); }
    if (route === '/v2/graph') return answer(200, { objects: [
      { id: SITE, members: world.members[SITE], view: { display_name: "Egregore's Echoes", roles: world.roles, parts: world.parts.map(x => ({ ...x })), face: { mark: 'data URL, 46303 chars', header: { logo: '', banner: '' } } } },
      { id: HOST, members: world.members[HOST], view: { publication: world.pub } },
      ...Object.keys(world.members).filter(id => id !== SITE && id !== HOST).map(id => ({ id, name: world.names[id], members: world.members[id], view: {} }))] });
    // As account.rs's Ask::Add: a site's token adds no one.
    if (route === '/v2/add') return answer(400, "a site's token adds no one");
    if (route === '/v2/batch') return answer(200, { made: ['x'.repeat(64)], refused: null });
    return answer(200, { delta: 'd' });
  } catch (e) { calls.push(['REFUSED', String(e.message)]); answer(401, String(e.message)); }
});
base = `http://127.0.0.1:${await listen(door)}`;

// The Site as the console's first run leaves it: the Arc on every roster, admitter of the Site
// and of each room, the rooms marked, the Face published.
const done = () => ({ scope: SITE, roles: [[ARC, 'admitter']], pub: { slug: 'egregore', publisher: ARC },
  members: { [SITE]: [ME, ARC], [HOST]: [ME, ARC], [HR]: [ME], [R]: [ME, ARC], [S]: [ME, ARC], [F]: [ME, ARC] },
  names: { [HR]: 'Healing Resistance', [R]: 'Resources', [S]: 'Skills, Time & Services', [F]: 'Financial Support' },
  parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }, { part: R, role: 'room', choice: 'resources' }, { part: S, role: 'room', choice: 'skills' }, { part: F, role: 'room', choice: 'financial' }] });
// The Site before any setup: the owner alone on each roster.
const fresh = () => ({ scope: SITE, roles: [], pub: null, members: { [SITE]: [ME], [HOST]: [ME], [HR]: [ME] }, names: { [HR]: 'Healing Resistance' },
  parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }] });

function registration(port, site = SITE) {
  const f = path.join(tmp, `clients-${port}-${site.slice(0, 4)}.json`);
  fs.writeFileSync(f, JSON.stringify({ [CLIENT]: { name: '', callbacks: [`http://127.0.0.1:${port}/callback`], origins: [], site } }));
  return f;
}

// The CLI, run as `make site-setup` runs it; `window(url)` plays the browser once it names the window.
function setup({ mark = TINY, clients, window }) {
  return new Promise(res => {
    const c = spawn(process.execPath, [CLI, 'egregore', mark], { env: { ...process.env, DOOR: base, ARC_BUNDLE_URL: ARC_URL, SITE_SETUP_OPEN: 'true', SITE_SETUP_CLIENTS: clients } });
    let out = '', err = '', opened = false;
    c.stdout.on('data', d => { out += d; });
    c.stderr.on('data', d => {
      err += d;
      const m = /the window: (\S+)\n/.exec(err);
      if (m && !opened) { opened = true; window(new URL(m[1])); }
    });
    const kill = setTimeout(() => c.kill(), 60000);
    c.on('close', code => { clearTimeout(kill); res({ code, out, err, opened }); });
  });
}
// The window, done: the Door gives a code for the challenge and redirects to the callback.
const signedIn = (extra = '') => async (u) => {
  const p = u.searchParams;
  assert.deepEqual([u.origin + u.pathname, p.get('client'), p.get('code_challenge_method')], [base + '/signin', CLIENT, 'S256'], 'the window, for the setup client, S256');
  assert.ok(/^[A-Za-z0-9_-]{43}$/.test(p.get('code_challenge')) && p.get('state').length >= 16, 'a challenge and a state');
  codeFor = { code: 'code-' + crypto.randomBytes(8).toString('hex'), callback: p.get('redirect_uri'), challenge: p.get('code_challenge') };
  const r = await fetch(`${p.get('redirect_uri')}?code=${codeFor.code}&state=${encodeURIComponent(p.get('state') + extra)}`);
  await r.text();
};
const routes = () => calls.map(([p, b]) => p + (b && b.op ? ' ' + b.op : ''));

// A RE-RUN (2.2.1): every step the console's run made is there; nothing is added or made again,
// nothing published again; each call carries the token and a fresh proof by its key; signed out.
{
  world = done(); calls = [];
  const port = await freePort();
  const r = await setup({ mark: BIG, clients: registration(port), window: signedIn() });
  assert.equal(r.code, 0, r.err);
  assert.deepEqual(routes(), ['/v2/token', '/v2/graph', '/v2/apply group.setClaimIssuer',
    '/v2/apply base.setRole', '/v2/apply base.setRole', '/v2/apply base.setRole',
    '/v2/apply group.setFace', '/v2/apply host.setMedia', '/v2/apply host.hydrate', '/v2/signout'], 'the re-run, the harmless re-applies alone, then signed out');
  assert.ok(!calls.some(([p]) => p === 'REFUSED'), JSON.stringify(calls.filter(([p]) => p === 'REFUSED')));
  for (const l of ['✓ signed in', '✓ the graph', 'already: the Arc added to the Site', "already: the Arc the Site's admitter", 'already: room Resources, for resources',
    'already: the Arc added to the Host', 'already: published at egregore', '✓ the mark', '✓ signed out']) assert.ok(r.out.includes(l + '\n'), l + ' in:\n' + r.out);
  assert.equal(bundlesServed, 5, 'five bundles, fetched before the window');
  // The big mark, drawn smaller with sips: under the limit, a JPEG (it has no alpha), no side over 1024.
  const media = calls.find(([, b]) => b && b.op === 'host.setMedia')[1].args;
  assert.equal(media.mediaMime, 'image/jpeg');
  assert.ok(media.media.length <= 156000, media.media.length + ' base64');
  const drawn = path.join(tmp, 'drawn.jpg'); fs.writeFileSync(drawn, Buffer.from(media.media, 'base64'));
  const dims = execFileSync('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', drawn], { encoding: 'utf8' });
  const [w, h] = [/pixelWidth: (\d+)/, /pixelHeight: (\d+)/].map(x => +x.exec(dims)[1]);
  assert.ok(Math.max(w, h) <= 1024 && Math.abs(w / h - 1.5) < 0.01, `drawn smaller, its shape kept: ${w}x${h}`);
  assert.ok(!/\bey[A-Za-z0-9_-]{20,}\./.test(r.out + r.err) && !issued.some(t => (r.out + r.err).includes(t)), 'no proof or token printed');
}
// The alpha mark: a PNG at every side is over the limit, so the JPEG the snippet asks for is what goes.
{
  world = done(); calls = [];
  const r = await setup({ mark: BIG_ALPHA, clients: registration(await freePort()), window: signedIn() });
  assert.equal(r.code, 0, r.err);
  const media = calls.find(([, b]) => b && b.op === 'host.setMedia')[1].args;
  assert.equal(media.mediaMime, 'image/jpeg');
  assert.ok(media.media.length <= 156000, media.media.length + ' base64');
}
// A mark within the limit goes as it is.
{
  world = done(); calls = [];
  const r = await setup({ mark: TINY, clients: registration(await freePort()), window: signedIn() });
  assert.equal(r.code, 0, r.err);
  const media = calls.find(([, b]) => b && b.op === 'host.setMedia')[1].args;
  assert.deepEqual([media.mediaMime, media.media], ['image/png', fs.readFileSync(TINY).toString('base64')], 'the file, unchanged');
}
// A FIRST RUN stops at the Site's add, a site's token adding no one, with nothing written: the
// graph read, the refused add, and the sign-out.
{
  world = fresh(); calls = [];
  const r = await setup({ clients: registration(await freePort()), window: signedIn() });
  assert.equal(r.code, 1, r.out);
  assert.match(r.err, /the Arc added to the Site: \/v2\/add 400 a site's token adds no one/);
  assert.deepEqual(routes(), ['/v2/token', '/v2/graph', '/v2/add', '/v2/signout'], 'nothing written before the add');
}
// Cancel in the window: no code, no token, nothing called.
{
  world = done(); calls = [];
  const r = await setup({ clients: registration(await freePort()), window: async (u) => {
    await (await fetch(`${u.searchParams.get('redirect_uri')}?error=access_denied&state=${encodeURIComponent(u.searchParams.get('state'))}`)).text();
  } });
  assert.equal(r.code, 1); assert.match(r.err, /the window answered access_denied/);
  assert.deepEqual(calls, [], 'the Door is not called');
}
// A callback with another state is refused, and its code is not traded.
{
  world = done(); calls = [];
  const r = await setup({ clients: registration(await freePort()), window: signedIn('x') });
  assert.equal(r.code, 1); assert.match(r.err, /another state/);
  assert.deepEqual(calls, [], 'no code traded');
  codeFor = null;
}
// The callback's port taken: it stops, naming it, before a bundle is fetched or a window opened.
{
  const held = http.createServer(); const port = await listen(held);
  const before = bundlesServed;
  const r = await setup({ clients: registration(port), window: () => assert.fail('a window opened') });
  held.close();
  assert.equal(r.code, 1); assert.ok(r.err.includes(`127.0.0.1:${port} is taken`), r.err);
  assert.equal(bundlesServed, before, 'no bundle spent'); assert.equal(r.opened, false);
}
// A registration for another Site than the snippet's: refused before anything.
{
  const before = bundlesServed;
  const r = await setup({ clients: registration(await freePort(), '9'.repeat(64)), window: () => assert.fail('a window opened') });
  assert.equal(r.code, 1); assert.match(r.err, /registered for 9{64}, the snippet's Site is 71c104ed/);
  assert.equal(bundlesServed, before);
}
arc.close(); door.close();
process.stdout.write("site-setup: a re-run adds and makes nothing, each call its token and a fresh proof, signed out; a first run stops at the Site's add, nothing written; the mark drawn smaller with sips; Cancel, another state, a taken port and another Site refused\n");
