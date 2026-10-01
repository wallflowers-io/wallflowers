/* audience-refusal.mjs — the device refuses to sign for an Arc that is not the
 * Arc it is talking to.
 *
 *   node app/web/smoke/audience-refusal.mjs
 *
 * WHAT IT PROVES, AND WHY NOTHING ELSE CAN. The audience is inside the bytes a
 * device signs so that a signature captured by one Arc cannot be replayed at
 * another — and the Arc DECLARES its own audience, in the challenge. A device
 * that signs whatever it is told makes the whole property rest on the Arc being
 * honest about its own name: a hostile origin declares `audience:
 * "arc.wallflowers.io"`, collects a signature over that string, and replays it
 * at the real arc.
 *
 * `pacific-account.js::headers` now checks. That check cannot be unit-tested
 * usefully — it is one comparison — and it cannot be caught by the Python suite,
 * which never runs the client. What is worth pinning is that the refusal FIRES,
 * in a real browser, against a real service, and does not fire in the normal
 * case. So this flips the running Arc's declared audience and asserts both.
 *
 * NOT WIRED INTO ANY CI, and it needs the gatetest stack up (`./drive.sh`) plus
 * docker, because it recreates the auth container twice. It leaves the container
 * as it found it, with its volume intact.
 *
 * WHY THIS BECAME CHECKABLE ONLY NOW. While the pages and the Arc shared an
 * origin the audience and the page host were the same string by accident, so the
 * comparison was a tautology. Giving the Arc its own hostname is what separates
 * them — and what makes the check mean something.
 */
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const ORIGIN = 'https://localhost:8793';
// smoke/ -> web/ -> app/ -> product/ -> the workspace; the site repo is business/website.
const SOON_ENV = fileURLToPath(new URL('../../../../business/website/site/soon/.env', import.meta.url));
const PW = execSync(
  `grep '^SITE_PW=' '${SOON_ENV}' | cut -d= -f2-`, { encoding: 'utf8' }).trim();

const vol = () => execSync(
  "docker inspect kenjin-auth --format '{{range .Mounts}}{{.Name}}{{end}}'",
  { encoding: 'utf8' }).trim();

function declareAudience(origin) {
  const v = vol();
  execSync('docker rm -f kenjin-auth', { stdio: 'ignore' });
  execSync(`docker run -d --name kenjin-auth --network gatetest -v ${v}:/data ` +
           `-e DB_PATH=/data/auth.db -e PORT=8000 -e PUBLIC_ORIGIN=${origin} ` +
           `kenjin-auth:gatetest uvicorn app.main:app --host 0.0.0.0 --port 8000`,
           { stdio: 'ignore' });
  execSync('sleep 3');
}

async function attempt() {
  const b = await chromium.launch({ args: ['--ignore-certificate-errors'] });
  try {
    const ctx = await b.newContext({ ignoreHTTPSErrors: true });
    const p = await ctx.newPage();
    const cdp = await ctx.newCDPSession(p);
    await cdp.send('WebAuthn.enable');
    await cdp.send('WebAuthn.addVirtualAuthenticator', { options: {
      protocol: 'ctap2', ctap2Version: 'ctap2_1', transport: 'internal',
      hasResidentKey: true, hasUserVerification: true, hasPrf: true,
      isUserVerified: true, automaticPresenceSimulation: true } });
    await p.goto(`${ORIGIN}/lock`, { waitUntil: 'domcontentloaded' }).catch(() => {});
    await p.evaluate(async pw => { await fetch('/unlock', { method: 'POST',
      headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ pw }) }); }, PW);
    await p.goto(`${ORIGIN}/signin`, { waitUntil: 'networkidle' });
    await p.waitForTimeout(1200);
    return await p.evaluate(async () => {
      const a = new PacificAccount({ corePath: 'assets/pacific/core/core_wasm_bg.wasm' });
      try { await a.create({ label: 'audience probe' }); return { made: true, why: '' }; }
      catch (e) { return { made: false, why: e.message }; }
    });
  } finally { await b.close(); }
}

let failed = 0;
const say = (ok, line) => { if (!ok) failed++; console.log(`${ok ? '  ok  ' : 'FAIL  '}${line}`); };

declareAudience('https://arc.attacker.example');
const lying = await attempt();
say(!lying.made, `a lying Arc is refused${lying.made ? ' — IT WAS NOT' : ''}`);
say(/calls itself/.test(lying.why) && /refusing to sign/.test(lying.why),
    `the refusal names both hosts: ${lying.why.slice(0, 90)}…`);

declareAudience(ORIGIN);
const honest = await attempt();
say(honest.made, `an honest Arc still works${honest.made ? '' : ` — ${honest.why}`}`);

console.log(failed ? `\n${failed} failed` : '\nall passed · the audience binding is a real check');
process.exit(failed ? 1 : 0);
