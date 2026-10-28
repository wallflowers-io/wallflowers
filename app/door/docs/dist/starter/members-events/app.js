/*
 * WallFlowers Community API example: Members and events.
 *
 * A club's events page, built on the starter (same shape: two files, no build). Signed in, it
 *   1. lists the community's upcoming events     the community's public page (no session), and
 *                                                any event this session made (GET /v2/graph)
 *   2. lets a member answer Going or Not going   POST /v2/apply: group.rsvp on the community
 *   3. lists the community's members beside them GET /v2/members
 *
 *   python3 -m http.server 5173 --bind 127.0.0.1      then open http://localhost:5173/
 *
 * Fill in CONFIG. Every route is in the Community API reference: https://docs.wallflowers.io/.
 */

export const CONFIG = {
  /** WallFlowers' Door: where signin.js and the API are served. */
  door: 'https://app.wallflowers.io',
  /** signin.js's sha384, as WallFlowers publishes it for the current Door release. */
  integrity: 'sha384-tlelF9i6GEcF6TboulAVcsSZoIVw6WnnBFG/cvUqcASGz3MuwBrF8jtAh5f5RfW/',
  /** Your client id: on your own machine, the shared local client where the release has one. It
      signs in only a community's owner and admins (its Site step offers no other): for members
      to sign in and answer, register this page's own client (Register your app). */
  client: 'hackathon-local',
  /** The callback registered for your client, character for character: this page. */
  callback: location.origin + location.pathname,
  /** Your community's address, as in wallflowers.io/<address> (signup shows it). A session
      can't read it, so it lives here: the public page is found by it. */
  address: 'your-community-address',
  /** Where public pages are served. */
  arc: 'https://arc.wallflowers.io',
};

const $ = (id) => document.getElementById(id);

/* ── sign in, as the starter does: signin.js from the Door, pinned by its hash ── */

function loadSignIn() {
  return new Promise((resolve, reject) => {
    if (window.WallFlowers) return resolve(window.WallFlowers);
    const script = Object.assign(document.createElement('script'), { src: `${CONFIG.door}/v2/signin.js`, integrity: CONFIG.integrity, crossOrigin: 'anonymous' });
    script.onload = () => resolve(window.WallFlowers);
    script.onerror = () => reject(new Error('signin.js did not load: on localhost, use client hackathon-local on a port it covers; any other address must be registered (Register your app, on the docs). Otherwise check CONFIG.door and CONFIG.integrity.'));
    document.head.append(script);
  });
}

async function main() {
  const wallflowers = await loadSignIn();
  const { client, callback } = CONFIG;
  // Back from the Door's window (?code&state), or a session kept from before.
  const session = new URLSearchParams(location.search).has('state')
    ? await wallflowers.finish({ client, callback })
    : await wallflowers.current({ client });
  if (!session) {
    $('signin').hidden = false;
    $('signin').onclick = () => wallflowers.signIn({ client, callback });
    return;
  }
  run(session);
}

/* ── the community: its events, the member's answers, its members ─────────────── */

function run(session) {
  let site = null;      // the community, from /v2/graph; its view holds every answer
  let me = null;        // the signed-in member's key, from /v2/members
  let publicItems = {}; // the public page's items, read at most every 30 s
  let readAt = 0;

  $('signout').hidden = false;
  $('signout').onclick = async () => { stop(); await session.signOut(); location.reload(); };

  async function json(path) {
    const res = await session.fetch(path);
    if (res.status === 401) throw new Error('Your session has ended. Reload to sign in again.');
    if (!res.ok) throw new Error(`${path}: ${res.status} ${await res.text()}`); // WallFlowers' own sentence
    return res.json();
  }

  // The public page lists every event the community published, the same for everyone. A
  // community not published yet answers 404 (from an Arc that sends it without a CORS header,
  // a failed fetch): then the page has only the events this session made.
  async function readPublic() {
    if (CONFIG.address === 'your-community-address' || Date.now() - readAt < 30_000) return;
    readAt = Date.now();
    try {
      const res = await fetch(`${CONFIG.arc}/v1/face/${encodeURIComponent(CONFIG.address)}/items`);
      if (res.ok) publicItems = (await res.json()).items ?? {};
    } catch { /* not published yet */ }
  }

  async function refresh() {
    try {
      const [m, graph] = await Promise.all([json('/v2/members'), json('/v2/graph'), readPublic()]);
      me = m.me;
      // Members as people are shown them: the owner first. A name, never a key.
      $('members').replaceChildren(...m.members.map((p) => li(`${p.name || 'New member'} · ${p.role}`)));

      site = graph.objects.find((o) => o.id === session.site);
      if (!site) return say('This account is not a member of this community.');
      $('community').textContent = site.view?.display_name || 'Your community';

      // Events by id: the public page's (op arguments by the model's names), then any this
      // session made that the community lists as its own (its `created` links).
      const events = new Map();
      for (const [key, it] of Object.entries(publicItems)) {
        if (key.startsWith('event:')) events.set(key.slice(6), { title: it.title, start: it.startMs, end: it.endMs, venue: it.venue, status: it.status, allDay: it.allDay });
      }
      const created = new Set((site.view?.affiliations ?? []).filter((a) => a.rel === 'created').map((a) => a.peer));
      for (const o of graph.objects) {
        const v = o.view ?? {};
        if (o.kind === 'event' && created.has(o.id)) events.set(o.id, { title: v.title, start: v.start_ms, end: v.end_ms, venue: v.venue, status: v.status, allDay: v.all_day });
      }

      // Upcoming: not yet over, soonest first. The answers and their counts are the community's.
      const answers = site.view?.rsvps ?? {}, counts = site.view?.rsvp_counts ?? {};
      const upcoming = [...events].filter(([, e]) => (e.end || e.start) > Date.now()).sort(([, a], [, b]) => a.start - b.start);
      $('events').replaceChildren(...(upcoming.length
        ? upcoming.map(([id, e]) => eventItem(id, e, answers[id]?.[me]?.status, counts[id]?.going ?? 0))
        : [li('No upcoming events.')]));
      say('');
      $('app').hidden = false;
    } catch (err) {
      say(err.message);
    }
  }

  // One event: when, what, where, how many are going, and this member's answer, pressed.
  function eventItem(id, e, mine, going) {
    const when = new Date(e.start).toLocaleString([], e.allDay ? { dateStyle: 'medium' } : { dateStyle: 'medium', timeStyle: 'short' }); // dates alone: no time
    const item = li(`${when} · ${e.title}${e.venue ? ` · ${e.venue}` : ''} · ${going} going`);
    if (e.status === 'cancelled') { item.append(' · cancelled'); return item; }
    const buttons = document.createElement('span');
    for (const [status, label] of [['going', 'Going'], ['declined', 'Not going']]) {
      const b = Object.assign(document.createElement('button'), { type: 'button', textContent: label });
      b.setAttribute('aria-pressed', String(mine === status));
      b.onclick = () => answer(id, status);
      buttons.append(b);
    }
    item.append(buttons);
    return item;
  }

  // Answer: one op on the community, not the event. A member is on no event's roster, so the
  // answers live in the community's log, where every member reads them. A later answer
  // replaces the earlier. WallFlowers adds the gen: never send one.
  async function answer(event, status) {
    const res = await session.fetch('/v2/apply', {
      method: 'POST',
      body: JSON.stringify({ object: site.id, op: 'group.rsvp', args: { event, status, at: Date.now() } }),
    });
    if (res.status === 401) return say('Your session has ended. Reload to sign in again.');
    if (!res.ok) return say(`Not answered: ${await res.text()}`); // e.g. answers closed, or the event is full
    refresh(); // don't wait for the stream: show it now
  }

  // One change stream for the page: it says only that something changed, so read again,
  // once a burst has settled. signin.js reopens it after a drop; onEnded means the session ended.
  let timer = 0;
  const stop = session.events(() => { clearTimeout(timer); timer = setTimeout(refresh, 300); }, () => say('Your session has ended. Reload to sign in again.'));
  refresh();
}

function li(text) { return Object.assign(document.createElement('li'), { textContent: text }); } // text, never HTML
function say(text) { $('status').textContent = text; $('status').hidden = !text; }

main().catch((err) => say(err.message));
