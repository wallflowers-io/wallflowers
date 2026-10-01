/*
 * WallFlowers Community API: the smallest working page.
 *
 * Sign in with WallFlowers, read the community your site is registered to, list its
 * rooms, and post a message. No build step, no dependencies: two files, this one and
 * index.html. Serve the folder from an origin WallFlowers registered as your client,
 * with this page's exact URL registered as your callback:
 *
 *   python3 -m http.server 5173 --bind 127.0.0.1      then open http://localhost:5173/
 *
 * Fill in CONFIG, below. Everything else follows the Community API reference:
 * https://docs.wallflowers.io/.
 */

export const CONFIG = {
  /** WallFlowers' Door: where signin.js and the API are served. */
  door: 'https://app.wallflowers.io',
  /** signin.js's sha384, as WallFlowers publishes it for the current Door release. */
  integrity: 'sha384-tlelF9i6GEcF6TboulAVcsSZoIVw6WnnBFG/cvUqcASGz3MuwBrF8jtAh5f5RfW/',
  /** Your client id: on your own machine, the shared hackathon client where the release has one;
      otherwise the one WallFlowers registered for you. */
  client: 'hackathon-local',
  /** The callback registered for your client, character for character: this page. */
  callback: location.origin + location.pathname,
};

const $ = (id) => document.getElementById(id);
const bare = (pk) => String(pk ?? '').replace(/^ed25519:/, '').toLowerCase();

/* ── the sign-in script, from the Door, pinned by its hash ───────────────── */

function loadSignIn() {
  return new Promise((resolve, reject) => {
    if (window.WallFlowers) return resolve(window.WallFlowers);
    const script = document.createElement('script');
    script.src = `${CONFIG.door}/v2/signin.js`;
    script.integrity = CONFIG.integrity;
    script.crossOrigin = 'anonymous';
    script.onload = () => (window.WallFlowers ? resolve(window.WallFlowers) : reject(new Error('signin.js loaded without WallFlowers')));
    // Most often the page's own address: the script loads with crossorigin, and WallFlowers
    // answers only origins it knows, so any other is blocked by CORS before it runs.
    script.onerror = () => reject(new Error('signin.js did not load. Most likely this page\'s address (' + location.origin + ') isn\'t allowed yet: on localhost, use client hackathon-local on a port it covers; any other address must be registered (Register your app, on the docs). Otherwise, check CONFIG.door, and CONFIG.integrity against the current release.'));
    document.head.append(script);
  });
}

/* ── signed in, or not ───────────────────────────────────────────────────── */

async function main() {
  if (CONFIG.client === 'your-client-id') return say('Set CONFIG.client (and check CONFIG.callback) in app.js first.');
  const wallflowers = await loadSignIn();
  const { client, callback } = CONFIG;

  // Back from the Door's window: the address carries ?code and ?state (or ?error).
  // finish() trades them for a session, and takes them off the address.
  let session;
  if (new URLSearchParams(location.search).has('state')) {
    session = await wallflowers.finish({ client, callback });
    if (!session) say('Sign-in cancelled.');
  } else {
    session = await wallflowers.current({ client });
  }

  if (!session) {
    $('signin').hidden = false;
    $('signin').onclick = () => wallflowers.signIn({ client, callback });
    return;
  }
  run(session);
}

/* ── the community ───────────────────────────────────────────────────────── */

function run(session) {
  let graph = null;
  let chosen = null; // the room being shown
  let timer = 0;
  const began = Date.now();

  $('signout').hidden = false;
  $('signout').onclick = async () => {
    clearInterval(net);
    stop();
    await session.signOut();
    location.reload();
  };

  // Everything this session can read, in one answer.
  async function refresh() {
    const res = await session.fetch('/v2/graph');
    if (res.status === 401) return signedOut();
    if (!res.ok) return say(`GET /v2/graph: ${res.status} ${await res.text()}`);
    graph = await res.json();
    draw();
  }

  // One change stream for the page, shared by everything on it. It says only that
  // something changed (a version, or '' after it reopened from a gap), so read
  // /v2/graph again, once a burst of changes has settled. signin.js reopens a stream
  // that drops by itself; a 401 means the session is over.
  const stop = session.events(
    () => {
      clearTimeout(timer);
      timer = setTimeout(refresh, 300);
    },
    () => signedOut(),
  );

  // A slow net: while the page is visible, read again every 15 s whatever the stream
  // says, in case a stream ended quietly.
  const net = setInterval(() => { if (document.visibilityState === 'visible') refresh(); }, 15_000);

  function signedOut() {
    clearInterval(net);
    stop();
    say('Your session has ended. Sign in again.');
    $('signout').hidden = true;
    $('app').hidden = true;
    main();
  }

  // Open a room, as "Rooms and threads" does: a forum with its name, made a part of the community
  // with role "room", in one batch, so it is all or nothing.
  async function newRoom(site) {
    const name = prompt('Name the room')?.trim();
    if (!name) return;
    const at = Date.now();
    const res = await session.fetch('/v2/batch', {
      method: 'POST',
      body: JSON.stringify({ steps: [
        { do: 'mint', kind: 'forum', draft: { name } },
        { do: 'apply', object: site, op: 'base.setPart', args: { part: { $step: 0 }, role: 'room', at } },
        { do: 'apply', object: { $step: 0 }, op: 'base.setParent', args: { parent: site, role: 'room', at } },
      ] }),
    });
    if (!res.ok) return say(await res.text());
    refresh();
  }

  function draw() {
    const objects = graph.objects ?? [];
    const byId = new Map(objects.map((o) => [o.id, o]));
    const site = byId.get(session.site);
    if (!site) {
      // a brand-new member's session can take a few seconds to reach the community; after
      // 20 s it's no wait: this account isn't one of the community's members
      if (Date.now() - began < 20_000) {
        setTimeout(refresh, 2000);
        return say('Reaching the community… A new member\'s first sign-in can take a few seconds.');
      }
      return say('This account isn\'t a member of this community, so the page can\'t reach it. Sign out, and sign in with an account that is: see "Members and roles" in the docs.');
    }
    if (site.folds === false) return say(`The community can't be read here: ${site.why ?? ''}`);

    // Names come from the members' cards, on the community's view and on each room's.
    // A key is never a name: anyone without a card is "New member".
    const me = bare(graph.me?.pk);
    const cards = { ...(site.view?.profiles ?? {}) };
    for (const o of objects) for (const [key, card] of Object.entries(o.view?.profiles ?? {})) cards[key] ??= card;
    const nameOf = (key) => cards[key]?.name?.trim() || (key === me ? graph.me?.display_name : null) || 'New member';

    // The rooms: the community's parts with role "room", in their order, that this
    // member reaches (a room the graph doesn't hold isn't theirs). One that can't be read
    // here (folds: false) is shown as unavailable, with why, never dropped.
    const reached = (site.view?.parts ?? [])
      .filter((p) => p.role === 'room')
      .sort((a, b) => (a.at ?? 0) - (b.at ?? 0))
      .map((p) => byId.get(p.part))
      .filter((o) => o && o.kind === 'forum');
    const rooms = reached.filter((o) => o.folds !== false);
    const unavailable = reached.filter((o) => o.folds === false);
    // A member who has just joined may see no rooms until the owner's app admits them.
    if (!rooms.length && Date.now() - began < 15_000) setTimeout(refresh, 1500);

    say('');
    $('app').hidden = false;
    $('community').textContent = site.view?.display_name || site.name || 'Your community';
    $('who').textContent = `Signed in as ${nameOf(me)}`;

    const list = $('rooms');
    list.replaceChildren(
      ...rooms.map((room) => {
        const item = document.createElement('li');
        const button = document.createElement('button');
        button.type = 'button';
        button.textContent = `${room.name} · ${(room.members ?? []).length} members`;
        button.setAttribute('aria-pressed', String(room.id === chosen));
        button.onclick = () => {
          chosen = room.id;
          draw();
        };
        item.append(button);
        return item;
      }),
    );
    for (const room of unavailable) {
      list.append(Object.assign(document.createElement('li'), { textContent: `${room.name ?? 'A room'}: unavailable here (${room.why ?? 'no reason given'})` }));
    }
    if (!reached.length) {
      const empty = Object.assign(document.createElement('li'), { textContent: 'No rooms yet. ' });
      // A new community has none: its owner may open the first one from here.
      if (site.owner === me) {
        const add = Object.assign(document.createElement('button'), { type: 'button', textContent: 'New room' });
        add.onclick = () => newRoom(site.id);
        empty.append(add);
      }
      list.replaceChildren(empty);
    }

    const room = rooms.find((r) => r.id === chosen) ?? rooms[0];
    chosen = room?.id ?? null;
    $('room').hidden = !room;
    if (!room) return;
    $('room-name').textContent = room.name;

    // A message is known by its author and gen; the newest last.
    const messages = [...(room.view?.messages ?? [])].sort((a, b) => (a.ts || 0) - (b.ts || 0) || a.gen - b.gen).slice(-30);
    $('messages').replaceChildren(
      ...messages.map((m) => {
        const item = document.createElement('li');
        const who = document.createElement('b');
        who.textContent = nameOf(m.author);
        const text = document.createElement('span');
        text.textContent = m.text; // text, never HTML
        item.append(who, ' ', text);
        return item;
      }),
    );
  }

  // Post: one op on the room, as the signed-in member. WallFlowers adds the gen.
  $('post').onsubmit = async (e) => {
    e.preventDefault();
    const text = $('text').value.trim();
    if (!text || !chosen) return;
    $('send').disabled = true;
    const res = await session.fetch('/v2/apply', {
      method: 'POST',
      body: JSON.stringify({ object: chosen, op: 'forum.post', args: { text, ts: Date.now() } }),
    });
    $('send').disabled = false;
    if (res.status === 401) return signedOut();
    if (!res.ok) return say(`Not posted: ${await res.text()}`); // show WallFlowers' sentence; don't parse it
    $('text').value = '';
    refresh(); // don't wait for the stream: show it now
  };

  refresh();
}

function say(text) {
  $('status').textContent = text;
  $('status').hidden = !text;
}

main().catch((err) => say(err.message));
