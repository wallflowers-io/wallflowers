/* E8's client: the egregore site's generic door (site/lib/social/door.ts, imported as it
   ships) on a DPoP-bound token (SEC-39; mdr/door.md §5, §6).

     node app/e2e/site_api.mjs <door.ts> '<json>'
       json: {door, public, code, verifier, client, redirect_uri, site, op, args, kind, draft,
              outside, cookie, args2, settle_ms}

   The proof is built here from RFC 9449, not taken from the Door's own client, so the
   Door is held to a second reading of it. The key is P-256 and not extractable. Prints
   one JSON object: the token's type and scope, and each answer door.ts gave. */
import { pathToFileURL } from 'node:url';

const [doorTs, raw] = process.argv.slice(2);
const a = JSON.parse(raw);
const { doorApi } = await import(pathToFileURL(doorTs).href);

const b64u = (b) => Buffer.from(b).toString('base64url');
const utf8 = (s) => new TextEncoder().encode(s);
const kp = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
const { x, y } = await crypto.subtle.exportKey('jwk', kp.publicKey);

async function proof(htm, htu, token) {
  const h = b64u(JSON.stringify({ typ: 'dpop+jwt', alg: 'ES256', jwk: { kty: 'EC', crv: 'P-256', x, y } }));
  const claims = { jti: crypto.randomUUID(), htm, htu, iat: Math.floor(Date.now() / 1000) };
  if (token) claims.ath = b64u(await crypto.subtle.digest('SHA-256', utf8(token)));
  const p = b64u(JSON.stringify(claims));
  const sig = await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, kp.privateKey, utf8(`${h}.${p}`));
  return `${h}.${p}.${b64u(sig)}`;
}

const out = {};
const r = await fetch(a.door + '/v2/token', {
  method: 'POST',
  headers: { 'content-type': 'application/json', dpop: await proof('POST', a.public + '/v2/token') },
  body: JSON.stringify({ code: a.code, code_verifier: a.verifier, client: a.client, redirect_uri: a.redirect_uri }),
});
if (!r.ok) {
  out.token = { status: r.status, said: (await r.text()).slice(0, 300) };
} else {
  const t = await r.json();
  out.token = { type: t.token_type, scope: t.scope, expires_in: t.expires_in, cache: r.headers.get('cache-control') };
  const session = {
    async fetch(path, init = {}) {
      const headers = new Headers(init.headers);
      headers.set('authorization', `DPoP ${t.access_token}`);
      headers.set('dpop', await proof((init.method || 'GET').toUpperCase(), a.public + path, t.access_token));
      return fetch(a.door + path, { ...init, headers });
    },
  };
  const api = doorApi(session);
  out.before = await api.fold();
  out.minted = await api.mint(a.kind, a.draft);
  out.wrote = await api.author(a.site, a.op, a.args);
  out.after = await api.fold();

  // SCOPE (RA-10, NC-41): `outside` is the same person's object, not the Site's. The
  // token's stream is counted while a webapp session (`cookie`) writes outside the Site,
  // then while the token writes inside it: the second is the control for the first.
  if (a.outside) {
    // The Door answers the stream 200 whatever the session said (fwd_stream), so the
    // body is read as SSE: each event's name and data, and any line that is not SSE.
    const events = [];
    const foreign = [];
    const stop = new AbortController();
    const es = await fetch(a.door + '/v2/events', {
      headers: { authorization: `DPoP ${t.access_token}`, dpop: await proof('GET', a.public + '/v2/events', t.access_token) },
      signal: stop.signal,
    });
    out.stream = { status: es.status, type: es.headers.get('content-type') };
    (async () => {
      const reader = es.body.getReader();
      const dec = new TextDecoder();
      let buf = '';
      let ev = { event: 'message', data: [] };
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          let i;
          while ((i = buf.indexOf('\n')) >= 0) {
            const line = buf.slice(0, i).replace(/\r$/, '');
            buf = buf.slice(i + 1);
            const m = /^(event|data|id|retry):\s?(.*)$/.exec(line);
            if (line === '') {
              if (ev.data.length) events.push({ event: ev.event, data: ev.data.join('\n') });
              ev = { event: 'message', data: [] };
            } else if (line.startsWith(':')) {
            } else if (m && m[1] === 'event') ev.event = m[2];
            else if (m && m[1] === 'data') ev.data.push(m[2]);
            else if (!m) foreign.push(line.slice(0, 300));
          }
        }
      } catch {}
    })();
    const settle = () => new Promise((ok) => setTimeout(ok, a.settle_ms));
    await settle();
    const e0 = events.length;
    const w = await fetch(a.door + '/v2/apply', {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie: a.cookie, origin: a.public },
      body: JSON.stringify({ object: a.outside, op: a.op, args: a.args }),
    });
    out.outside_by_cookie = w.status;
    await settle();
    const e1 = events.length;
    out.inside_again = await api.author(a.site, a.op, a.args2);
    await settle();
    out.events = { before: events.slice(0, e0), outside: events.slice(e0, e1), inside: events.slice(e1), foreign };
    out.outside_by_token = await api.author(a.outside, a.op, a.args);
    stop.abort();
  }
}
process.stdout.write(JSON.stringify(out));
